//! Commands for `assist::groups`: rooms, their messages and runs, shares,
//! conversation context and app-owned worktrees. Owner: worker W5.
//!
//! Member turns are real turns of the roster bots, each in its own session
//! (`"<room>~<bot>"`), started through [`AppDispatcher`] and streamed on
//! `llm://turn` like a direct chat. Room changes are announced on
//! `assist://group` as `{ room_id, kind }` so the open room reloads.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use omniget_core::core::assist::{
    ctx::Scope,
    db, external_config as xc,
    groups::{self, store, tasks, worktree, RoomDraft},
};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::commands::llm::chat::{forward_turn, TurnSummary};
use crate::commands::llm::ensure_wired;
use crate::AppState;

pub const EVENT_GROUP: &str = "assist://group";

fn emit(app: &AppHandle, room: &str, kind: &str, extra: Value) {
    let mut payload = json!({ "room_id": room, "kind": kind });
    if let (Some(p), Value::Object(e)) = (payload.as_object_mut(), extra) {
        p.extend(e);
    }
    let _ = app.emit(EVENT_GROUP, payload);
}

/// Starts member turns for the room engine and reports their end back.
pub struct AppDispatcher {
    app: AppHandle,
}

#[async_trait::async_trait]
impl tasks::RoomDispatcher for AppDispatcher {
    async fn start(&self, room: &str, bot: &str, input: String) -> Result<String, String> {
        // LOCAL_USER dispatcher: never an external room or a derived bot (H1).
        db::global()?.with(|c| Ok(xc::deny_local_dispatch(c, Some(room), bot)))??;
        let manager = self.app.state::<AppState>().llm.clone();
        ensure_wired(&self.app);
        let conversation = groups::participant_id(room, bot);
        let (request_id, _cancel, stream) = manager.turn_stream(&conversation, bot, &input).await?;
        let db = db::global()?;
        let app = self.app.clone();
        let room_id = room.to_string();
        let run_id = request_id.clone();
        let cancel_manager = manager.clone();
        let done = Box::new(move |sum: TurnSummary| {
            let end = if sum.cancelled {
                tasks::RunEnd::Cancelled
            } else if sum.error.is_some() && sum.text.trim().is_empty() {
                tasks::RunEnd::Failed
            } else {
                tasks::RunEnd::Completed
            };
            match tasks::finish_run(
                &db,
                &run_id,
                end,
                &sum.text,
                sum.error.as_deref(),
                sum.tokens,
            ) {
                Ok((_, cascade)) => {
                    for r in cascade {
                        let _ = cancel_manager.cancel(&r);
                    }
                }
                Err(e) => tracing::warn!("[groups] finishing {run_id}: {e}"),
            }
            emit(&app, &room_id, "run_finished", json!({ "run_id": run_id }));
        });
        forward_turn(
            self.app.clone(),
            manager,
            bot.to_string(),
            conversation,
            input,
            request_id.clone(),
            stream,
            Some(done),
        );
        emit(
            &self.app,
            room,
            "run_started",
            json!({ "run_id": request_id, "bot_id": bot }),
        );
        Ok(request_id)
    }

    fn cancel(&self, run_id: &str) {
        let _ = self.app.state::<AppState>().llm.cancel(run_id);
    }
}

/// Installs the dispatcher the first time a room is used in this session.
fn dispatcher(app: &AppHandle) -> Arc<dyn tasks::RoomDispatcher> {
    if let Some(d) = tasks::dispatcher() {
        return d;
    }
    let d: Arc<dyn tasks::RoomDispatcher> = Arc::new(AppDispatcher { app: app.clone() });
    tasks::set_dispatcher(d.clone());
    d
}

fn names(state: &AppState) -> HashMap<String, String> {
    state
        .llm
        .roster()
        .into_iter()
        .map(|a| (a.id, a.name))
        .collect()
}

fn to_value<T: serde::Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

/// External rooms are read-only locally; local rooms take no derived bot.
fn local_edit_allowed(room: Option<&str>, draft: Option<&RoomDraft>) -> Result<(), String> {
    let db = db::global()?;
    db.with(|c| {
        Ok((|| {
            if let Some(room) = room {
                xc::deny_local_dispatch(c, Some(room), "")?;
            }
            for m in draft.map(|d| d.members.as_slice()).unwrap_or_default() {
                xc::deny_local_dispatch(c, None, &m.bot)?;
            }
            Ok::<(), String>(())
        })())
    })?
}

/// Rooms with, for an external one, `external: { principal, grantId,
/// grantActive, retired, readOnly }` so the UI shows it as a read-only
/// object of that client instead of a normal room.
#[tauri::command]
pub async fn assist_group_list() -> Result<Value, String> {
    let db = db::global()?;
    let marks = xc::room_marks(&db)?;
    let mut rooms = to_value(store::list_rooms(&db)?)?;
    if let Value::Array(items) = &mut rooms {
        for item in items {
            let mark = item
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| marks.get(id))
                .cloned();
            if let (Some(obj), Some(mark)) = (item.as_object_mut(), mark) {
                obj.insert("external".into(), to_value(mark)?);
            }
        }
    }
    Ok(rooms)
}

#[tauri::command]
pub async fn assist_group_create(
    app: AppHandle,
    state: State<'_, AppState>,
    draft: RoomDraft,
) -> Result<Value, String> {
    let known = names(&state);
    if let Some(m) = draft.members.iter().find(|m| !known.contains_key(&m.bot)) {
        return Err(format!("{}: no bot {}", groups::ERR_GROUP, m.bot));
    }
    local_edit_allowed(None, Some(&draft))?;
    let room = store::create_room(&*db::global()?, &draft)?;
    emit(&app, &room.id, "room", Value::Null);
    to_value(room)
}

#[tauri::command]
pub async fn assist_group_update(
    app: AppHandle,
    state: State<'_, AppState>,
    room_id: String,
    draft: RoomDraft,
) -> Result<Value, String> {
    let known = names(&state);
    if let Some(m) = draft.members.iter().find(|m| !known.contains_key(&m.bot)) {
        return Err(format!("{}: no bot {}", groups::ERR_GROUP, m.bot));
    }
    local_edit_allowed(Some(&room_id), Some(&draft))?;
    let room = store::update_room(&*db::global()?, &room_id, &draft)?;
    emit(&app, &room.id, "room", Value::Null);
    to_value(room)
}

#[tauri::command]
pub async fn assist_group_delete(app: AppHandle, room_id: String) -> Result<Value, String> {
    let db = db::global()?;
    tasks::cancel_room(&db, &room_id, Some(dispatcher(&app).as_ref()))?;
    store::delete_room(&db, &room_id)?;
    emit(&app, &room_id, "deleted", Value::Null);
    Ok(json!({ "ok": true }))
}

/// The room transcript plus its runs and delegated tasks (activity).
#[tauri::command]
pub async fn assist_group_messages(room_id: String) -> Result<Value, String> {
    let db = db::global()?;
    Ok(json!({
        "room": to_value(store::get_room(&db, &room_id)?)?,
        "messages": to_value(store::messages(&db, &room_id)?)?,
        "runs": to_value(tasks::runs(&db, &room_id)?)?,
        "tasks": to_value(tasks::tasks(&db, &room_id)?)?,
    }))
}

/// A user message: persisted, routed (mention → that member; none →
/// coordinator or default bot), and only the routed members start a run.
#[tauri::command]
pub async fn assist_group_send(
    app: AppHandle,
    state: State<'_, AppState>,
    room_id: String,
    text: String,
) -> Result<Value, String> {
    let db = db::global()?;
    let d = dispatcher(&app);
    let out = tasks::send_user_message(&db, d.as_ref(), &room_id, &text, &names(&state)).await?;
    emit(&app, &room_id, "message", Value::Null);
    to_value(out)
}

/// Cancels what is still running in the room; finished answers stay.
#[tauri::command]
pub async fn assist_group_cancel(app: AppHandle, room_id: String) -> Result<Value, String> {
    let db = db::global()?;
    let out = tasks::cancel_room(&db, &room_id, Some(dispatcher(&app).as_ref()))?;
    emit(&app, &room_id, "cancelled", Value::Null);
    to_value(out)
}

#[tauri::command]
pub async fn assist_group_shares(room_id: String) -> Result<Value, String> {
    to_value(store::shares(&*db::global()?, &room_id)?)
}

/// The user shares a scope (`user` = personal profile, `bot:<id>` = a bot's
/// private memory) or one record into a room. Only this command creates a
/// share; no tool can.
#[tauri::command]
pub async fn assist_group_share(
    app: AppHandle,
    room_id: String,
    scope: String,
    record_id: Option<String>,
    note: Option<String>,
) -> Result<Value, String> {
    let scope =
        Scope::parse(&scope).ok_or_else(|| format!("{}: unknown scope", groups::ERR_GROUP))?;
    // Local memory is never shared into an external room.
    local_edit_allowed(Some(&room_id), None)?;
    let share = store::add_share(
        &*db::global()?,
        &room_id,
        &scope,
        record_id.as_deref(),
        note.as_deref().unwrap_or(""),
    )?;
    emit(&app, &room_id, "share", Value::Null);
    to_value(share)
}

#[tauri::command]
pub async fn assist_group_unshare(
    app: AppHandle,
    room_id: String,
    share_id: String,
) -> Result<Value, String> {
    store::revoke_share(&*db::global()?, &share_id)?;
    emit(&app, &room_id, "share", Value::Null);
    Ok(json!({ "ok": true }))
}

/// `{ kind: "projectless" | "project", path }` of one conversation.
#[tauri::command]
pub async fn assist_conversation_context(conversation_id: String) -> Result<Value, String> {
    let (kind, path) = groups::context_of(&crate::llm_manager::sanitize_id(&conversation_id));
    Ok(json!({
        "kind": kind,
        "path": path.map(|p| p.to_string_lossy().to_string()),
    }))
}

/// A separate worktree of the conversation's project, only on an explicit
/// request, marked as the app's. With `bind`, the conversation moves to it.
#[tauri::command]
pub async fn assist_worktree_create(
    conversation_id: String,
    bind: Option<bool>,
) -> Result<Value, String> {
    let conv = crate::llm_manager::sanitize_id(&conversation_id);
    let (_, repo) = groups::context_of(&conv);
    let repo = repo.ok_or_else(|| format!("{}: pick a project folder first", groups::ERR_GROUP))?;
    let base: PathBuf = omniget_core::core::llm::roster_store::llm_dir()
        .ok_or_else(|| format!("{}: no app data folder", groups::ERR_GROUP))?
        .join("worktrees");
    let wt = worktree::create(&*db::global()?, &repo, &base, Some(&conv))?;
    if bind.unwrap_or(false) {
        groups::set_context(&conv, Some(wt.path.clone()))?;
    }
    to_value(wt)
}

#[tauri::command]
pub async fn assist_worktree_list() -> Result<Value, String> {
    to_value(worktree::list(&*db::global()?)?)
}

/// Removes a worktree the app created; any other folder is refused.
#[tauri::command]
pub async fn assist_worktree_remove(path: String) -> Result<Value, String> {
    worktree::remove(&*db::global()?, std::path::Path::new(&path))?;
    Ok(json!({ "ok": true }))
}
