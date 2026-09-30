//! Command dispatcher shared by the app (`telegram_call`) and the CLI.
//! Names and argument shapes are the old plugin's, so the UI can call them
//! unchanged; args accept both `snake_case` and `camelCase`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use base64::Engine as _;
use serde_json::json;
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

use super::client::auth::{QrPollStatus, VerifyError};
use super::client::events::{self, DownloadReporter};
use super::client::{self, api, auth, thumbnail_cache};
use super::host;

/// Every command `dispatch` understands.
pub const COMMANDS: &[&str] = &[
    "telegram_check_session",
    "telegram_qr_start",
    "telegram_qr_poll",
    "telegram_send_code",
    "telegram_verify_code",
    "telegram_verify_2fa",
    "telegram_logout",
    "telegram_takeout_start",
    "telegram_takeout_finish",
    "telegram_set_mute",
    "telegram_toggle_pin",
    "telegram_set_archived",
    "telegram_reorder_pinned",
    "telegram_full_channel_info",
    "telegram_cancel_op",
    "telegram_list_chats",
    "telegram_create_folder",
    "telegram_delete_channel",
    "telegram_leave_channel",
    "telegram_delete_history",
    "telegram_list_participants",
    "telegram_set_blocked",
    "telegram_report_peer",
    "telegram_rename_channel",
    "telegram_restore_peer_hashes",
    "telegram_list_chats_page",
    "telegram_get_self",
    "telegram_stream_info",
    "telegram_delete_messages",
    "telegram_forward_messages",
    "telegram_edit_caption",
    "telegram_search_global",
    "telegram_search_global_hits",
    "telegram_upload_media",
    "telegram_list_media",
    "telegram_diag_list_media",
    "telegram_expand_album",
    "telegram_download_media",
    "telegram_download_batch",
    "telegram_cancel_batch",
    "telegram_get_thumbnail",
    "telegram_search_media",
    "telegram_get_chat_photo",
    "telegram_clear_thumbnail_cache",
    "telegram_perf_get",
    "telegram_clone_start",
    "telegram_clone_status",
    "telegram_clone_list",
    "telegram_clone_pause",
    "telegram_clone_cancel",
    "telegram_clone_delete",
    "telegram_sync_state",
    "telegram_sync_now",
    "telegram_sync_settings_set",
    "telegram_accounts_list",
    "telegram_accounts_save_current",
    "telegram_accounts_restore",
    "telegram_accounts_remove",
    "telegram_accounts_rename",
    "telegram_accounts_backup_now",
    "telegram_accounts_list_backups",
    "telegram_bandwidth_stats",
    "telegram_bandwidth_set_quota",
    "telegram_bandwidth_reset",
    "telegram_perf_set",
    "telegram_resolve_links",
    "telegram_download_links",
    "telegram_export_chat",
    "telegram_settings_get",
    "telegram_list_commands",
    "telegram_stream_url",
    "telegram_playback_get",
    "telegram_playback_set",
    "telegram_playback_recent",
    "telegram_open_external",
    "telegram_external_players",
    "telegram_watch_list",
    "telegram_watch_add",
    "telegram_watch_update",
    "telegram_watch_remove",
    "telegram_watch_run",
    "telegram_watch_settings",
    "telegram_download_chat",
    "telegram_preview_chat_download",
    "telegram_upload_paths",
    "telegram_realtime_get",
    "telegram_realtime_set",
    "telegram_scan_chat",
    "telegram_scan_cancel",
    "telegram_index_stats",
    "telegram_index_clear",
    "telegram_dedupe_stats",
    "telegram_dedupe_set",
    "telegram_dedupe_clear",
    "telegram_transcribe",
    "telegram_voice_transcribe_native",
    "telegram_transcript_get",
    "telegram_asr_status",
    "telegram_asr_install",
    "telegram_add_to_watch",
    "telegram_approval_list",
    "telegram_approval_answer",
];

/// Commands that work without a connected client (login, local files,
/// settings). Every other command first restores a saved session.
const OFFLINE_COMMANDS: &[&str] = &[
    "telegram_check_session",
    "telegram_qr_start",
    "telegram_qr_poll",
    "telegram_send_code",
    "telegram_verify_code",
    "telegram_verify_2fa",
    "telegram_logout",
    "telegram_cancel_op",
    "telegram_cancel_batch",
    "telegram_clear_thumbnail_cache",
    "telegram_perf_get",
    "telegram_perf_set",
    "telegram_clone_status",
    "telegram_clone_list",
    "telegram_clone_pause",
    "telegram_clone_cancel",
    "telegram_clone_delete",
    "telegram_sync_state",
    "telegram_sync_settings_set",
    "telegram_accounts_list",
    "telegram_accounts_save_current",
    "telegram_accounts_restore",
    "telegram_accounts_remove",
    "telegram_accounts_rename",
    "telegram_accounts_backup_now",
    "telegram_accounts_list_backups",
    "telegram_bandwidth_stats",
    "telegram_bandwidth_set_quota",
    "telegram_bandwidth_reset",
    "telegram_settings_get",
    "telegram_list_commands",
    "telegram_stream_info",
    "telegram_playback_get",
    "telegram_playback_set",
    "telegram_playback_recent",
    "telegram_open_external",
    "telegram_external_players",
    "telegram_watch_list",
    "telegram_watch_update",
    "telegram_watch_remove",
    "telegram_watch_settings",
    "telegram_realtime_get",
    "telegram_realtime_set",
    "telegram_scan_cancel",
    "telegram_index_stats",
    "telegram_index_clear",
    "telegram_dedupe_stats",
    "telegram_dedupe_set",
    "telegram_dedupe_clear",
    "telegram_transcript_get",
    "telegram_asr_status",
    "telegram_asr_install",
    "telegram_add_to_watch",
    "telegram_approval_list",
    "telegram_approval_answer",
];

#[derive(Clone, serde::Deserialize)]
struct BatchItem {
    message_id: i32,
    file_name: String,
    #[serde(default)]
    file_size: u64,
    // Optional message data, used by name templates.
    #[serde(default)]
    date: i64,
    #[serde(default)]
    media_type: Option<String>,
    #[serde(default)]
    caption: Option<String>,
    #[serde(default)]
    grouped_id: Option<i64>,
}

impl BatchItem {
    fn as_media_item(&self) -> api::TelegramMediaItem {
        api::TelegramMediaItem {
            message_id: self.message_id,
            file_name: self.file_name.clone(),
            file_size: self.file_size,
            media_type: self.media_type.clone().unwrap_or_else(|| "document".into()),
            date: self.date,
            webpage: None,
            caption: self.caption.clone(),
            grouped_id: self.grouped_id,
        }
    }
}

#[derive(Default)]
struct BatchCounters {
    completed: AtomicU32,
    failed: AtomicU32,
    skipped: AtomicU32,
    deleted: AtomicU32,
    resumed: AtomicU32,
    /// Counted in `skipped` too.
    duplicates: AtomicU32,
}

impl BatchCounters {
    fn done(&self) -> u32 {
        self.completed.load(Ordering::Relaxed)
            + self.failed.load(Ordering::Relaxed)
            + self.skipped.load(Ordering::Relaxed)
            + self.deleted.load(Ordering::Relaxed)
            + self.resumed.load(Ordering::Relaxed)
    }
}

fn opt_string(args: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| args.get(*k))
        .and_then(|v| match v {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Array(a) => Some(
                a.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .filter(|s| !s.trim().is_empty())
}

/// `{chatId, chatType}` (or snake_case).
fn chat_of(args: &serde_json::Value) -> Result<(i64, String), String> {
    let chat_id = args
        .get("chatId")
        .or(args.get("chat_id"))
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .ok_or("missing 'chatId'")?;
    let chat_type = args
        .get("chatType")
        .or(args.get("chat_type"))
        .and_then(|v| v.as_str())
        .ok_or("missing 'chatType'")?
        .to_string();
    Ok((chat_id, chat_type))
}

fn message_id_of(args: &serde_json::Value) -> Result<i32, String> {
    args.get("messageId")
        .or(args.get("message_id"))
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)
        .ok_or_else(|| "missing 'messageId'".into())
}

fn num_of(args: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|k| args.get(*k))
        .and_then(|v| v.as_f64())
}

fn opt_bool(args: &serde_json::Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|k| args.get(*k))
        .and_then(|v| v.as_bool())
}

fn bool_of(args: &serde_json::Value, keys: &[&str]) -> bool {
    keys.iter()
        .find_map(|k| args.get(*k))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// What a whole-chat download (or its preview) would fetch.
struct ChatPlan {
    title: String,
    root: std::path::PathBuf,
    items: Vec<api::TelegramMediaItem>,
    planned: Vec<client::batch::Planned>,
    scanned: usize,
    skip_same: bool,
    dedupe: bool,
    scan_id: String,
    /// File uid of each item (same order as `items`).
    uids: Vec<Option<String>>,
}

/// Brings the chat's index up to date as far as `filter` needs and decides
/// every file's path, oldest first. Emits `telegram-chat-scan {scan_id,
/// chat_id, scanned, matched, done}` per page.
async fn plan_chat_download(
    session: &auth::TelegramSessionHandle,
    args: &serde_json::Value,
    fix_ext: bool,
) -> Result<ChatPlan, String> {
    let (chat_id, chat_type) = chat_of(args)?;
    let title = opt_string(args, &["chatTitle", "chat_title"]).unwrap_or_default();
    let filter = client::history::MediaFilter::from_json(args.get("filter"))?;
    let template = opt_string(args, &["template"]);
    let root = opt_string(args, &["outputDir", "output_dir"])
        .map(std::path::PathBuf::from)
        .unwrap_or_else(client::download_dir);
    let skip_same = args
        .get("skipSame")
        .or(args.get("skip_same"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let _ = fix_ext;
    let dedupe = opt_bool(args, &["dedupe"]).unwrap_or_else(client::dedupe::enabled);
    let scan_id =
        opt_string(args, &["scanId", "scan_id"]).unwrap_or_else(client::index::new_scan_id);
    let max_scan = num_of(args, &["maxScan", "max_scan"])
        .unwrap_or(0.0)
        .max(0.0) as usize;
    let (rows, res) = client::index::scan_filtered(
        session,
        chat_id,
        &chat_type,
        &filter,
        &scan_id,
        max_scan,
        bool_of(args, &["takeout"]),
        0,
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut rows = rows;
    rows.reverse();
    let uids: Vec<Option<String>> = rows.iter().map(|(_, u)| u.clone()).collect();
    let items: Vec<api::TelegramMediaItem> = rows.into_iter().map(|(it, _)| it).collect();
    let mut planned: Vec<client::batch::Planned> = items
        .iter()
        .map(|it| {
            client::batch::plan_item(
                &root,
                template.as_deref(),
                Some(client::template::CHAT_DEFAULT),
                chat_id,
                &chat_type,
                &title,
                it,
            )
        })
        .collect();
    client::batch::dedupe_paths(&mut planned);
    Ok(ChatPlan {
        title,
        root,
        items,
        planned,
        scanned: res.scanned,
        skip_same,
        dedupe,
        scan_id,
        uids,
    })
}

/// Runs one command. The work runs on its own task, boxed, so the large
/// grammers futures never sit on the caller's stack.
pub async fn dispatch(
    command: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let name = command.clone();
    tokio::spawn(Box::pin(dispatch_inner(command, args)))
        .await
        .unwrap_or_else(|e| Err(join_error_message(&name, e)))
}

fn join_error_message(command: &str, e: tokio::task::JoinError) -> String {
    if e.is_panic() {
        let payload = e.into_panic();
        let msg = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown panic".to_string());
        tracing::error!("[telegram] command {} panicked: {}", command, msg);
        format!(
            "Telegram command '{}' failed unexpectedly: {}",
            command, msg
        )
    } else {
        format!(
            "Telegram command '{}' was cancelled before it finished",
            command
        )
    }
}

async fn dispatch_inner(
    command: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let eng = client::engine();
    let session = eng.session.clone();
    let active = eng.active.clone();
    let stream_server_slot = eng.stream_server.clone();
    let fix_ext = client::fix_file_extensions();
    let concurrent = client::concurrent_downloads();
    if !OFFLINE_COMMANDS.contains(&command.as_str()) {
        client::restore_session(&session).await;
    }
    match command.as_str() {
        "telegram_check_session" => {
            let r = auth::check_session(&session)
                .await
                .map_err(|e| e.to_string())?;
            Ok(serde_json::to_value(r).map_err(|e| e.to_string())?)
        }

        "telegram_qr_start" => {
            let r = auth::qr_login_start(&session)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "svg": r.qr_svg, "expires": r.expires }))
        }

        "telegram_qr_poll" => match auth::qr_login_poll(&session).await {
            Ok(QrPollStatus::Waiting) => Ok(json!("waiting")),
            Ok(QrPollStatus::Success { phone }) => Ok(json!(format!("success:{}", phone))),
            Ok(QrPollStatus::PasswordRequired { hint }) => {
                if hint.is_empty() {
                    Ok(json!("password_required"))
                } else {
                    Ok(json!(format!("password_required:{}", hint)))
                }
            }
            Err(e) => Err(e.to_string()),
        },

        "telegram_send_code" => {
            let phone: String =
                serde_json::from_value(args.get("phone").cloned().ok_or("missing 'phone'")?)
                    .map_err(|e| e.to_string())?;
            auth::send_code(&session, &phone)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!(null))
        }

        "telegram_verify_code" => {
            let code: String =
                serde_json::from_value(args.get("code").cloned().ok_or("missing 'code'")?)
                    .map_err(|e| e.to_string())?;
            match auth::verify_code(&session, &code).await {
                Ok(phone) => Ok(json!(phone)),
                Err(VerifyError::InvalidCode) => Err("invalid_code".into()),
                Err(VerifyError::PasswordRequired { hint }) => {
                    Err(format!("password_required:{}", hint))
                }
                Err(VerifyError::NoSession) => Err("no_session".into()),
                Err(VerifyError::Other(msg)) => Err(msg),
            }
        }

        "telegram_verify_2fa" => {
            let password: String =
                serde_json::from_value(args.get("password").cloned().ok_or("missing 'password'")?)
                    .map_err(|e| e.to_string())?;
            let r = auth::verify_password(&session, &password)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!(r))
        }

        "telegram_logout" => {
            thumbnail_cache::clear_cache().await;
            auth::logout(&session).await.map_err(|e| e.to_string())?;
            Ok(json!(null))
        }

        "telegram_takeout_start" => {
            let id = api::init_takeout(&session)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "takeout_id": id }))
        }

        "telegram_takeout_finish" => {
            let success: bool = args
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            api::finish_takeout(&session, success)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!(null))
        }

        "telegram_set_mute" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let mute_until: i64 = serde_json::from_value(
                args.get("mute_until")
                    .or(args.get("muteUntil"))
                    .cloned()
                    .unwrap_or(json!(0)),
            )
            .map_err(|e| e.to_string())?;
            api::set_mute(&session, chat_id, &chat_type, mute_until)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!(null))
        }

        "telegram_toggle_pin" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let pinned: bool = args
                .get("pinned")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            api::toggle_pin(&session, chat_id, &chat_type, pinned)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!(null))
        }

        "telegram_set_archived" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let archived: bool = args
                .get("archived")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            api::set_archived(&session, chat_id, &chat_type, archived)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!(null))
        }

        "telegram_reorder_pinned" => {
            let raw: Vec<serde_json::Value> =
                serde_json::from_value(args.get("items").cloned().ok_or("missing 'items'")?)
                    .map_err(|e| e.to_string())?;
            let items: Vec<(i64, String)> = raw
                .into_iter()
                .filter_map(|v| {
                    let cid = v
                        .get("chat_id")
                        .or(v.get("chatId"))
                        .and_then(|x| x.as_i64())?;
                    let ct = v
                        .get("chat_type")
                        .or(v.get("chatType"))
                        .and_then(|x| x.as_str())?
                        .to_string();
                    Some((cid, ct))
                })
                .collect();
            api::reorder_pinned(&session, items)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!(null))
        }

        "telegram_full_channel_info" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let r = api::full_channel_info(&session, chat_id, &chat_type)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }

        "telegram_cancel_op" => {
            let op_id: String = serde_json::from_value(
                args.get("op_id")
                    .or(args.get("opId"))
                    .cloned()
                    .ok_or("missing 'op_id'")?,
            )
            .map_err(|e| e.to_string())?;
            let g = session.lock().await;
            let map = g.op_cancellation_tokens.clone();
            drop(g);
            let cancelled = auth::cancel_op_token(&map, &op_id);
            if cancelled {
                tracing::info!("[tg-cancel] op_id={} cancelled", op_id);
            } else {
                tracing::debug!("[tg-cancel] op_id={} not found", op_id);
            }
            Ok(json!({ "cancelled": cancelled }))
        }

        "telegram_list_chats" => {
            let r = api::list_chats(&session).await.map_err(|e| e.to_string())?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }

        "telegram_create_folder" => {
            let name: String =
                serde_json::from_value(args.get("name").cloned().ok_or("missing 'name'")?)
                    .map_err(|e| e.to_string())?;
            let r = api::create_folder_channel(&session, &name)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }

        "telegram_delete_channel" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chatId")
                    .or(args.get("chat_id"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            api::delete_channel(&session, chat_id)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        }

        "telegram_leave_channel" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chatId")
                    .or(args.get("chat_id"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chatType")
                    .or(args.get("chat_type"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            api::leave_channel(&session, chat_id, &chat_type)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        }

        "telegram_delete_history" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chatId")
                    .or(args.get("chat_id"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chatType")
                    .or(args.get("chat_type"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let just_clear: bool = args
                .get("justClear")
                .or(args.get("just_clear"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let revoke: bool = args
                .get("revoke")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            api::delete_history(&session, chat_id, &chat_type, just_clear, revoke)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        }

        "telegram_list_participants" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chatId")
                    .or(args.get("chat_id"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let filter: String = args
                .get("filter")
                .and_then(|v| v.as_str())
                .unwrap_or("recent")
                .to_string();
            let offset: i32 = args.get("offset").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let limit: i32 = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(50) as i32;
            let search: Option<String> = args
                .get("search")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let r = api::list_participants(&session, chat_id, &filter, offset, limit, search)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }

        "telegram_set_blocked" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chatId")
                    .or(args.get("chat_id"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chatType")
                    .or(args.get("chat_type"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let blocked: bool = args
                .get("blocked")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            api::set_blocked(&session, chat_id, &chat_type, blocked)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        }

        "telegram_report_peer" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chatId")
                    .or(args.get("chat_id"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chatType")
                    .or(args.get("chat_type"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let message_ids: Vec<i32> = args
                .get("messageIds")
                .or(args.get("message_ids"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_i64().map(|n| n as i32))
                        .collect()
                })
                .unwrap_or_default();
            let option: Vec<u8> = args
                .get("option")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_i64().map(|n| n as u8))
                        .collect()
                })
                .unwrap_or_default();
            let message: String = args
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            api::report_peer(&session, chat_id, &chat_type, message_ids, option, message)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        }

        "telegram_rename_channel" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chatId")
                    .or(args.get("chat_id"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let title: String =
                serde_json::from_value(args.get("title").cloned().ok_or("missing 'title'")?)
                    .map_err(|e| e.to_string())?;
            api::rename_channel(&session, chat_id, title)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        }

        "telegram_restore_peer_hashes" => {
            let raw: Vec<Vec<i64>> =
                serde_json::from_value(args.get("items").cloned().ok_or("missing 'items'")?)
                    .map_err(|e| e.to_string())?;
            let items: Vec<(i64, i64)> = raw
                .into_iter()
                .filter_map(|v| {
                    if v.len() == 2 {
                        Some((v[0], v[1]))
                    } else {
                        None
                    }
                })
                .collect();
            api::restore_peer_hashes(&session, items)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        }

        "telegram_list_chats_page" => {
            let offset_date: i32 = args
                .get("offset_date")
                .or(args.get("offsetDate"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            let offset_id: i32 = args
                .get("offset_id")
                .or(args.get("offsetId"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            let limit: i32 = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(40) as i32;
            let offset_peer: Option<(i64, String, i64)> = args
                .get("offset_peer")
                .or(args.get("offsetPeer"))
                .and_then(|v| {
                    let id = v.get("peer_id").or(v.get("peerId"))?.as_i64()?;
                    let t = v
                        .get("peer_type")
                        .or(v.get("peerType"))?
                        .as_str()?
                        .to_string();
                    let h = v.get("peer_hash").or(v.get("peerHash"))?.as_i64()?;
                    Some((id, t, h))
                });
            let r = api::list_chats_page(&session, offset_date, offset_id, offset_peer, limit)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }

        "telegram_get_self" => {
            let r = api::get_self(&session).await.map_err(|e| e.to_string())?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }

        "telegram_stream_info" => {
            let slot = stream_server_slot.lock().await;
            let info = slot
                .as_ref()
                .ok_or_else(|| "stream server not ready".to_string())?
                .clone();
            drop(slot);
            let info_guard = info.lock().await;
            Ok(json!({
                "port": info_guard.port,
                "token": info_guard.token,
                "base_url": format!("http://127.0.0.1:{}", info_guard.port),
            }))
        }

        "telegram_delete_messages" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let message_ids: Vec<i32> = serde_json::from_value(
                args.get("message_ids")
                    .or(args.get("messageIds"))
                    .cloned()
                    .ok_or("missing 'messageIds'")?,
            )
            .map_err(|e| e.to_string())?;
            let revoke: bool = args.get("revoke").and_then(|v| v.as_bool()).unwrap_or(true);
            let n = api::delete_messages(&session, chat_id, &chat_type, message_ids, revoke)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "deleted": n }))
        }

        "telegram_forward_messages" => {
            let from_chat_id: i64 = serde_json::from_value(
                args.get("from_chat_id")
                    .or(args.get("fromChatId"))
                    .cloned()
                    .ok_or("missing 'fromChatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let from_chat_type: String = serde_json::from_value(
                args.get("from_chat_type")
                    .or(args.get("fromChatType"))
                    .cloned()
                    .ok_or("missing 'fromChatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let to_chat_id: i64 = serde_json::from_value(
                args.get("to_chat_id")
                    .or(args.get("toChatId"))
                    .cloned()
                    .ok_or("missing 'toChatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let to_chat_type: String = serde_json::from_value(
                args.get("to_chat_type")
                    .or(args.get("toChatType"))
                    .cloned()
                    .ok_or("missing 'toChatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let message_ids: Vec<i32> = serde_json::from_value(
                args.get("message_ids")
                    .or(args.get("messageIds"))
                    .cloned()
                    .ok_or("missing 'messageIds'")?,
            )
            .map_err(|e| e.to_string())?;
            let drop_author: bool = args
                .get("drop_author")
                .or(args.get("dropAuthor"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let drop_captions: bool = args
                .get("drop_captions")
                .or(args.get("dropCaptions"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            api::forward_messages(
                &session,
                from_chat_id,
                &from_chat_type,
                to_chat_id,
                &to_chat_type,
                message_ids,
                drop_author,
                drop_captions,
            )
            .await
            .map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        }

        "telegram_edit_caption" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let message_id: i32 = serde_json::from_value(
                args.get("message_id")
                    .or(args.get("messageId"))
                    .cloned()
                    .ok_or("missing 'messageId'")?,
            )
            .map_err(|e| e.to_string())?;
            let caption: String =
                serde_json::from_value(args.get("caption").cloned().ok_or("missing 'caption'")?)
                    .map_err(|e| e.to_string())?;
            api::edit_caption(&session, chat_id, &chat_type, message_id, caption)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        }

        "telegram_search_global" => {
            let query: String =
                serde_json::from_value(args.get("query").cloned().ok_or("missing 'query'")?)
                    .map_err(|e| e.to_string())?;
            let limit: u32 =
                serde_json::from_value(args.get("limit").cloned().unwrap_or(json!(50)))
                    .map_err(|e| e.to_string())?;
            let r = api::search_global(&session, &query, limit, fix_ext)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }

        "telegram_search_global_hits" => {
            let query: String =
                serde_json::from_value(args.get("query").cloned().ok_or("missing 'query'")?)
                    .map_err(|e| e.to_string())?;
            let limit: u32 =
                serde_json::from_value(args.get("limit").cloned().unwrap_or(json!(50)))
                    .map_err(|e| e.to_string())?;
            let r = api::search_global_hits(&session, &query, limit, fix_ext)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }

        "telegram_upload_media" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let file_path: String = serde_json::from_value(
                args.get("file_path")
                    .or(args.get("filePath"))
                    .cloned()
                    .ok_or("missing 'filePath'")?,
            )
            .map_err(|e| e.to_string())?;
            let caption: Option<String> = args
                .get("caption")
                .and_then(|v| serde_json::from_value(v.clone()).ok());

            let path = std::path::PathBuf::from(&file_path);
            let file_name = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".into());
            let upload_id = events::next_id();
            let title = file_name.clone();
            events::upload_progress(upload_id, &title, 0.0);

            tokio::spawn(async move {
                let result = Box::pin(api::upload_media(
                    &session,
                    chat_id,
                    &chat_type,
                    &path,
                    caption.as_deref(),
                ))
                .await;
                match result {
                    Ok(message_id) => {
                        events::upload_progress(upload_id, &title, 100.0);
                        events::upload_complete(upload_id, &title, true, None, Some(message_id));
                    }
                    Err(e) => {
                        events::upload_complete(upload_id, &title, false, Some(e.to_string()), None)
                    }
                }
            });

            Ok(json!({ "id": upload_id, "file_name": file_name }))
        }

        "telegram_list_media" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let media_type: Option<String> = args
                .get("media_type")
                .or(args.get("mediaType"))
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok());
            let offset: i32 =
                serde_json::from_value(args.get("offset").cloned().unwrap_or(json!(0)))
                    .map_err(|e| e.to_string())?;
            let limit: u32 =
                serde_json::from_value(args.get("limit").cloned().unwrap_or(json!(50)))
                    .map_err(|e| e.to_string())?;
            let r = api::list_media(
                &session,
                chat_id,
                &chat_type,
                media_type.as_deref(),
                offset,
                limit,
                fix_ext,
            )
            .await
            .map_err(|e| e.to_string())?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }

        "telegram_diag_list_media" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let r = api::diag_list_media(&session, chat_id, &chat_type)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }

        "telegram_expand_album" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let message_id: i32 = serde_json::from_value(
                args.get("message_id")
                    .or(args.get("messageId"))
                    .cloned()
                    .ok_or("missing 'messageId'")?,
            )
            .map_err(|e| e.to_string())?;
            let r = api::expand_album(&session, chat_id, &chat_type, message_id, fix_ext)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }

        "telegram_download_media" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let message_id: i32 = serde_json::from_value(
                args.get("message_id")
                    .or(args.get("messageId"))
                    .cloned()
                    .ok_or("missing 'messageId'")?,
            )
            .map_err(|e| e.to_string())?;
            let file_name: String = serde_json::from_value(
                args.get("file_name")
                    .or(args.get("fileName"))
                    .cloned()
                    .ok_or("missing 'fileName'")?,
            )
            .map_err(|e| e.to_string())?;
            let output_dir: String = opt_string(&args, &["output_dir", "outputDir"])
                .unwrap_or_else(|| client::download_dir().to_string_lossy().into_owned());
            let file_size: Option<u64> = args
                .get("file_size")
                .or(args.get("fileSize"))
                .and_then(|v| v.as_u64())
                .filter(|s| *s > 0);
            let op_id: Option<String> = args
                .get("op_id")
                .or(args.get("opId"))
                .and_then(|v| v.as_str().map(|s| s.to_string()));
            let output_path = match opt_string(&args, &["template"]) {
                Some(t) => {
                    let chat_title =
                        opt_string(&args, &["chat_title", "chatTitle"]).unwrap_or_default();
                    let mut item = client::history::fetch_items(
                        &session,
                        chat_id,
                        &chat_type,
                        &[message_id],
                        fix_ext,
                    )
                    .await
                    .ok()
                    .and_then(|mut m| m.remove(&message_id))
                    .unwrap_or_else(|| api::TelegramMediaItem {
                        message_id,
                        file_name: file_name.clone(),
                        file_size: file_size.unwrap_or(0),
                        media_type: "document".into(),
                        date: 0,
                        webpage: None,
                        caption: None,
                        grouped_id: None,
                    });
                    item.file_name = file_name.clone();
                    client::batch::plan_item(
                        std::path::Path::new(&output_dir),
                        Some(&t),
                        None,
                        chat_id,
                        &chat_type,
                        &chat_title,
                        &item,
                    )
                    .path
                }
                None => std::path::PathBuf::from(&output_dir).join(&file_name),
            };

            let download_id = events::next_id();
            let cancel_token = if let Some(ref oid) = op_id {
                let map = session.lock().await.op_cancellation_tokens.clone();
                auth::register_op_token(&map, oid)
            } else {
                CancellationToken::new()
            };
            {
                let mut a = active.lock().await;
                let key = format!("tg:{}:{}", chat_id, message_id);
                if a.values().any(|(u, _)| u == &key) {
                    return Err("Download already in progress".into());
                }
                a.insert(download_id, (key, cancel_token.clone()));
            }

            let file_name_clone = file_name.clone();
            tokio::spawn(async move {
                let mut rep = DownloadReporter::new(
                    download_id,
                    &file_name_clone,
                    "single",
                    None,
                    file_size,
                    Some(output_path.to_string_lossy().into()),
                );
                rep.progress(0.0);
                let (tx, mut rx) = mpsc::channel::<f64>(32);
                let pf = tokio::spawn(async move {
                    while let Some(p) = rx.recv().await {
                        rep.progress(p);
                    }
                    rep
                });
                let result = Box::pin(api::download_media_with_retry(
                    &session,
                    chat_id,
                    &chat_type,
                    message_id,
                    &output_path,
                    tx,
                    &cancel_token,
                ))
                .await;
                let rep = pf.await.ok();
                active.lock().await.remove(&download_id);
                if let Some(ref oid) = op_id {
                    let map = session.lock().await.op_cancellation_tokens.clone();
                    auth::unregister_op_token(&map, oid);
                }
                let Some(rep) = rep else { return };
                match result {
                    Ok(size) => {
                        client::bandwidth::record_bytes(&host::data_dir(), size);
                        rep.complete(
                            true,
                            None,
                            Some(output_path.to_string_lossy().into()),
                            Some(size),
                            Some(1),
                            None,
                        );
                    }
                    Err(e) => rep.complete(false, Some(e.to_string()), None, None, None, None),
                }
            });
            Ok(json!({ "id": download_id, "file_name": file_name }))
        }

        "telegram_download_batch" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_title: String = serde_json::from_value(
                args.get("chat_title")
                    .or(args.get("chatTitle"))
                    .cloned()
                    .ok_or("missing 'chatTitle'")?,
            )
            .map_err(|e| e.to_string())?;
            let items: Vec<BatchItem> =
                serde_json::from_value(args.get("items").cloned().ok_or("missing 'items'")?)
                    .map_err(|e| e.to_string())?;
            let output_dir: String = opt_string(&args, &["output_dir", "outputDir"])
                .unwrap_or_else(|| client::download_dir().to_string_lossy().into_owned());
            let resume_enabled: bool = args
                .get("resume")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let dedupe = opt_bool(&args, &["dedupe"]).unwrap_or_else(client::dedupe::enabled);
            let mut paths: HashMap<i32, std::path::PathBuf> = HashMap::new();
            if let Some(t) = opt_string(&args, &["template"]) {
                let need_meta =
                    client::template::needs_metadata(&t) && items.iter().any(|i| i.date == 0);
                let meta = if need_meta {
                    let ids: Vec<i32> = items.iter().map(|i| i.message_id).collect();
                    client::history::fetch_items(&session, chat_id, &chat_type, &ids, fix_ext)
                        .await
                        .unwrap_or_default()
                } else {
                    HashMap::new()
                };
                let mut planned: Vec<client::batch::Planned> = items
                    .iter()
                    .map(|i| {
                        let mut it = meta
                            .get(&i.message_id)
                            .cloned()
                            .unwrap_or_else(|| i.as_media_item());
                        it.file_name = i.file_name.clone();
                        client::batch::plan_item(
                            std::path::Path::new(&output_dir),
                            Some(&t),
                            None,
                            chat_id,
                            &chat_type,
                            &chat_title,
                            &it,
                        )
                    })
                    .collect();
                client::batch::dedupe_paths(&mut planned);
                paths = planned
                    .into_iter()
                    .map(|p| (p.message_id, p.path))
                    .collect();
            }
            let paths = Arc::new(paths);

            let batch_id = events::next_id();
            let cancel_token = CancellationToken::new();
            active.lock().await.insert(
                batch_id,
                (format!("tg-batch:{}", batch_id), cancel_token.clone()),
            );

            let total_files = items.len().max(1) as u32;
            let total_bytes: u64 = items.iter().map(|i| i.file_size).sum();
            let data_dir = host::data_dir();
            let fingerprint = if resume_enabled {
                let ids: Vec<i32> = items.iter().map(|i| i.message_id).collect();
                Some(client::resume::compute_fingerprint(chat_id, &ids))
            } else {
                None
            };
            let resume_completed: HashSet<i32> = match fingerprint {
                Some(ref fp) => {
                    client::resume::load_resume_state(&data_dir, fp)
                        .await
                        .completed
                }
                None => HashSet::new(),
            };

            let mut agg = DownloadReporter::new(
                batch_id,
                &chat_title,
                "batch",
                None,
                Some(total_bytes).filter(|b| *b > 0),
                Some(output_dir.clone()),
            );
            agg.progress(0.0);

            tokio::spawn(async move {
                let agg = Arc::new(tokio::sync::Mutex::new(agg));
                let sem = Arc::new(Semaphore::new(concurrent));
                let counters = Arc::new(BatchCounters::default());
                let resume_completed = Arc::new(resume_completed);
                let fingerprint = Arc::new(fingerprint);
                let data_dir = Arc::new(data_dir);
                let mut handles = Vec::new();

                for item in items {
                    let sem = sem.clone();
                    let session = session.clone();
                    let chat_type = chat_type.clone();
                    let output_dir = output_dir.clone();
                    let cancel = cancel_token.clone();
                    let counters = counters.clone();
                    let agg = agg.clone();
                    let resume_completed = resume_completed.clone();
                    let fingerprint = fingerprint.clone();
                    let data_dir = data_dir.clone();
                    let paths = paths.clone();

                    handles.push(tokio::spawn(async move {
                        if cancel.is_cancelled() { return; }
                        let tick = |agg: Arc<tokio::sync::Mutex<DownloadReporter>>, counters: Arc<BatchCounters>| async move {
                            let done = counters.done();
                            agg.lock().await.progress(done as f64 / total_files as f64 * 100.0);
                        };

                        if fingerprint.is_some() && resume_completed.contains(&item.message_id) {
                            counters.resumed.fetch_add(1, Ordering::Relaxed);
                            events::batch_file_status(batch_id, item.message_id, "resumed_skip", 100.0, None);
                            tick(agg, counters).await;
                            return;
                        }

                        let output_path = paths.get(&item.message_id).cloned().unwrap_or_else(|| std::path::PathBuf::from(&output_dir).join(&item.file_name));
                        if let Ok(meta) = std::fs::metadata(&output_path) {
                            if meta.len() > 0 && (item.file_size == 0 || meta.len() == item.file_size) {
                                counters.skipped.fetch_add(1, Ordering::Relaxed);
                                events::batch_file_status(batch_id, item.message_id, "skipped", 100.0, None);
                                tick(agg, counters).await;
                                return;
                            }
                        }

                        let _permit = tokio::select! { p = sem.acquire() => match p { Ok(p) => p, Err(_) => return }, _ = cancel.cancelled() => return };
                        if cancel.is_cancelled() { return; }

                        let mut rep = DownloadReporter::new(events::next_id(), &item.file_name, "item", Some(batch_id), Some(item.file_size).filter(|s| *s > 0), Some(output_path.to_string_lossy().into()));
                        events::batch_file_status(batch_id, item.message_id, "downloading", 0.0, None);
                        let (tx, mut rx) = mpsc::channel::<f64>(32);
                        let mid = item.message_id;
                        let pf = tokio::spawn(async move {
                            while let Some(p) = rx.recv().await {
                                rep.progress(p);
                                events::batch_file_status(batch_id, mid, "downloading", p, None);
                            }
                            rep
                        });

                        let result = Box::pin(api::download_media_with_retry_ex(&session, chat_id, &chat_type, item.message_id, &output_path, tx, &cancel, dedupe)).await;
                        let rep = pf.await.ok();

                        match result {
                            Err(e) if client::dedupe::duplicate_of(&e.to_string()).is_some() => {
                                let msg = e.to_string();
                                let existing = client::dedupe::duplicate_of(&msg).unwrap_or_default().to_string();
                                counters.skipped.fetch_add(1, Ordering::Relaxed);
                                counters.duplicates.fetch_add(1, Ordering::Relaxed);
                                events::batch_file_duplicate(batch_id, item.message_id, &existing);
                                if let Some(r) = rep { r.complete(true, None, Some(existing), None, Some(0), None); }
                            }
                            Ok(size) => {
                                counters.completed.fetch_add(1, Ordering::Relaxed);
                                client::bandwidth::record_bytes(&data_dir, size);
                                events::batch_file_status(batch_id, item.message_id, "done", 100.0, None);
                                if let Some(r) = rep { r.complete(true, None, Some(output_path.to_string_lossy().into()), Some(size), Some(1), None); }
                                if let Some(ref fp) = *fingerprint {
                                    if let Err(e) = client::resume::mark_completed(&data_dir, fp, item.message_id).await {
                                        tracing::warn!("[tg-resume] mark_completed failed: {}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                let err_str = e.to_string();
                                if err_str.contains("cancelled") {
                                    if let Some(r) = rep { r.complete(false, Some(err_str), None, None, None, None); }
                                    return;
                                }
                                let upper = err_str.to_uppercase();
                                let is_deleted = upper.contains("MESSAGE_ID_INVALID") || upper.contains("MESSAGEDELETED") || upper.contains("MESSAGE_DELETED");
                                if is_deleted {
                                    counters.deleted.fetch_add(1, Ordering::Relaxed);
                                    events::batch_file_status(batch_id, item.message_id, "skipped_deleted", 100.0, Some(err_str.clone()));
                                    if let Some(r) = rep { r.complete(false, Some(format!("skipped (deleted): {}", err_str)), None, None, None, None); }
                                } else {
                                    counters.failed.fetch_add(1, Ordering::Relaxed);
                                    events::batch_file_status(batch_id, item.message_id, "error", 0.0, Some(err_str.clone()));
                                    if let Some(r) = rep { r.complete(false, Some(err_str), None, None, None, None); }
                                }
                            }
                        }
                        tick(agg, counters).await;
                    }));
                }

                for h in handles {
                    let _ = h.await;
                }
                active.lock().await.remove(&batch_id);
                let done_count = counters.completed.load(Ordering::Relaxed);
                let fail_count = counters.failed.load(Ordering::Relaxed);
                let skip_count = counters.skipped.load(Ordering::Relaxed);
                let deleted_count = counters.deleted.load(Ordering::Relaxed);
                let resumed_count = counters.resumed.load(Ordering::Relaxed);
                let duplicate_count = counters.duplicates.load(Ordering::Relaxed);
                let mut error_summary: Vec<String> = Vec::new();
                if fail_count > 0 {
                    error_summary.push(format!("{} files failed", fail_count));
                }
                if deleted_count > 0 {
                    error_summary.push(format!(
                        "{} messages deleted/invalid (skipped)",
                        deleted_count
                    ));
                }
                let cancelled = cancel_token.is_cancelled();
                agg.lock().await.complete_with(
                    fail_count == 0 && !cancelled,
                    if cancelled {
                        Some("Batch cancelled".into())
                    } else if !error_summary.is_empty() {
                        Some(error_summary.join("; "))
                    } else {
                        None
                    },
                    Some(output_dir),
                    None,
                    Some(done_count + skip_count + resumed_count),
                    (deleted_count > 0).then_some(deleted_count),
                    json!({ "duplicate_count": duplicate_count }),
                );
            });
            Ok(json!(batch_id))
        }

        "telegram_cancel_batch" => {
            let batch_id: u64 = serde_json::from_value(
                args.get("batch_id")
                    .or(args.get("batchId"))
                    .cloned()
                    .ok_or("missing 'batchId'")?,
            )
            .map_err(|e| e.to_string())?;
            let a = active.lock().await;
            if let Some((key, token)) = a.get(&batch_id) {
                if key.starts_with("tg-batch:")
                    || key.starts_with("tg-export:")
                    || key.starts_with("tg-upload:")
                    || key.starts_with("tg:")
                {
                    token.cancel();
                    return Ok(json!(null));
                }
            }
            Err("Batch not found".into())
        }

        "telegram_get_thumbnail" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let message_id: i32 = serde_json::from_value(
                args.get("message_id")
                    .or(args.get("messageId"))
                    .cloned()
                    .ok_or("missing 'messageId'")?,
            )
            .map_err(|e| e.to_string())?;
            let data = thumbnail_cache::get_thumbnail(&session, chat_id, &chat_type, message_id)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!(
                base64::engine::general_purpose::STANDARD.encode(&data)
            ))
        }

        "telegram_search_media" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let query: String =
                serde_json::from_value(args.get("query").cloned().ok_or("missing 'query'")?)
                    .map_err(|e| e.to_string())?;
            let media_type: Option<String> = args
                .get("media_type")
                .or(args.get("mediaType"))
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok());
            let limit: u32 =
                serde_json::from_value(args.get("limit").cloned().unwrap_or(json!(50)))
                    .map_err(|e| e.to_string())?;
            let from_date = num_of(&args, &["fromDate", "from_date"]);
            let to_date = num_of(&args, &["toDate", "to_date"]);
            let min_size = num_of(&args, &["minSize", "min_size"]);
            let max_size = num_of(&args, &["maxSize", "max_size"]);
            let has_bounds = from_date.is_some()
                || to_date.is_some()
                || min_size.is_some()
                || max_size.is_some();
            if has_bounds || client::index::coverage(chat_id).complete {
                // Search the local index (caption and file name), reading
                // only what the index still lacks.
                let filter = client::history::MediaFilter {
                    media_types: media_type
                        .clone()
                        .filter(|t| !t.is_empty() && t != "all")
                        .map(|t| vec![t]),
                    from_date,
                    to_date,
                    min_size,
                    max_size,
                    keyword: Some(query.clone()).filter(|q| !q.trim().is_empty()),
                    limit: Some(limit.max(1) as u64),
                    ..Default::default()
                };
                let scan_id = opt_string(&args, &["scanId", "scan_id"])
                    .unwrap_or_else(client::index::new_scan_id);
                let max_scan = num_of(&args, &["maxScan", "max_scan"])
                    .unwrap_or(20_000.0)
                    .max(0.0) as usize;
                let (rows, _) = client::index::scan_filtered(
                    &session, chat_id, &chat_type, &filter, &scan_id, max_scan, false, 0,
                )
                .await
                .map_err(|e| e.to_string())?;
                let items: Vec<api::TelegramMediaItem> =
                    rows.into_iter().map(|(it, _)| it).collect();
                return serde_json::to_value(items).map_err(|e| e.to_string());
            }
            let r = api::search_media(
                &session,
                chat_id,
                &chat_type,
                &query,
                media_type.as_deref(),
                limit,
                fix_ext,
            )
            .await
            .map_err(|e| e.to_string())?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }

        "telegram_get_chat_photo" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let data = thumbnail_cache::get_chat_photo(&session, chat_id, &chat_type)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!(
                base64::engine::general_purpose::STANDARD.encode(&data)
            ))
        }

        "telegram_clear_thumbnail_cache" => {
            thumbnail_cache::clear_cache().await;
            Ok(json!(null))
        }

        "telegram_perf_get" => {
            let max_threads = client::parallel_download::get_max_threads_cap();
            let buckets = json!([
                { "min_mb": 0, "max_mb": 1, "threads": client::parallel_download::compute_thread_count(0) },
                { "min_mb": 1, "max_mb": 5, "threads": client::parallel_download::compute_thread_count(1024 * 1024 * 2) },
                { "min_mb": 5, "max_mb": 20, "threads": client::parallel_download::compute_thread_count(1024 * 1024 * 10) },
                { "min_mb": 20, "max_mb": 50, "threads": client::parallel_download::compute_thread_count(1024 * 1024 * 30) },
                { "min_mb": 50, "max_mb": 0, "threads": client::parallel_download::compute_thread_count(1024 * 1024 * 100) },
            ]);
            Ok(json!({
                "max_threads": max_threads,
                "max_part_size_kb": 1024,
                "buckets": buckets,
            }))
        }

        "telegram_clone_start" => {
            let source_id: i64 = serde_json::from_value(
                args.get("sourceId")
                    .or(args.get("source_id"))
                    .cloned()
                    .ok_or("missing 'sourceId'")?,
            )
            .map_err(|e| e.to_string())?;
            let source_type: String = serde_json::from_value(
                args.get("sourceType")
                    .or(args.get("source_type"))
                    .cloned()
                    .ok_or("missing 'sourceType'")?,
            )
            .map_err(|e| e.to_string())?;
            let source_title: String = args
                .get("sourceTitle")
                .or(args.get("source_title"))
                .and_then(|v| v.as_str())
                .unwrap_or("Source")
                .to_string();
            let dest_id_opt: Option<i64> = args
                .get("destId")
                .or(args.get("dest_id"))
                .and_then(|v| v.as_i64());
            let dest_type_opt: Option<String> = args
                .get("destType")
                .or(args.get("dest_type"))
                .and_then(|v| v.as_str().map(|s| s.to_string()));
            let dest_title_input: Option<String> = args
                .get("destTitle")
                .or(args.get("dest_title"))
                .and_then(|v| v.as_str().map(|s| s.to_string()));
            let resume_id: Option<String> = args
                .get("resumeId")
                .or(args.get("resume_id"))
                .and_then(|v| v.as_str().map(|s| s.to_string()));
            let opts: client::clone::CloneOptions = match args.get("options").cloned() {
                Some(v) => serde_json::from_value(v).unwrap_or_default(),
                None => client::clone::CloneOptions::default(),
            };

            let (dest_id, dest_type, dest_title) = if let Some(id) = dest_id_opt {
                (
                    id,
                    dest_type_opt.unwrap_or_else(|| "channel".to_string()),
                    dest_title_input
                        .clone()
                        .unwrap_or_else(|| format!("Channel {}", id)),
                )
            } else {
                let auto_title =
                    dest_title_input.unwrap_or_else(|| format!("{} - clone", source_title));
                let (id, _hash, title) =
                    client::clone::create_destination_channel(&session, &auto_title)
                        .await
                        .map_err(|e| e.to_string())?;
                (id, "channel".to_string(), title)
            };

            let session_id = client::clone::start_clone(
                session.clone(),
                host::data_dir(),
                tokio::runtime::Handle::current(),
                source_id,
                source_type,
                source_title,
                dest_id,
                dest_type.clone(),
                dest_title.clone(),
                opts,
                resume_id,
            )
            .await
            .map_err(|e| e.to_string())?;

            Ok(json!({
                "session_id": session_id,
                "dest_chat_id": dest_id,
                "dest_chat_type": dest_type,
                "dest_title": dest_title,
            }))
        }

        "telegram_clone_status" => {
            let id: String = serde_json::from_value(
                args.get("sessionId")
                    .or(args.get("session_id"))
                    .cloned()
                    .ok_or("missing 'sessionId'")?,
            )
            .map_err(|e| e.to_string())?;
            let pdd = host::data_dir();
            let s = client::clone::load_session(&pdd, &id).map_err(|e| e.to_string())?;
            serde_json::to_value(s).map_err(|e| e.to_string())
        }

        "telegram_clone_list" => {
            let pdd = host::data_dir();
            let list = client::clone::list_sessions(&pdd);
            serde_json::to_value(list).map_err(|e| e.to_string())
        }

        "telegram_clone_pause" => {
            let id: String = serde_json::from_value(
                args.get("sessionId")
                    .or(args.get("session_id"))
                    .cloned()
                    .ok_or("missing 'sessionId'")?,
            )
            .map_err(|e| e.to_string())?;
            let cancelled = client::clone::cancel_session(&id).await;
            Ok(json!({ "cancelled": cancelled }))
        }

        "telegram_clone_cancel" => {
            let id: String = serde_json::from_value(
                args.get("sessionId")
                    .or(args.get("session_id"))
                    .cloned()
                    .ok_or("missing 'sessionId'")?,
            )
            .map_err(|e| e.to_string())?;
            let _ = client::clone::cancel_session(&id).await;
            let pdd = host::data_dir();
            let mut s = client::clone::load_session(&pdd, &id).map_err(|e| e.to_string())?;
            s.status = "cancelled".to_string();
            client::clone::save_session(&pdd, &s).map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        }

        "telegram_clone_delete" => {
            let id: String = serde_json::from_value(
                args.get("sessionId")
                    .or(args.get("session_id"))
                    .cloned()
                    .ok_or("missing 'sessionId'")?,
            )
            .map_err(|e| e.to_string())?;
            let _ = client::clone::cancel_session(&id).await;
            let pdd = host::data_dir();
            client::clone::delete_session(&pdd, &id).map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        }

        "telegram_sync_state" => {
            Ok(serde_json::to_value(client::sync_state::snapshot()).map_err(|e| e.to_string())?)
        }

        "telegram_sync_now" => {
            let updated = client::sync_state::run_once(&session)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "updated": updated }))
        }

        "telegram_sync_settings_set" => {
            if let Some(enabled) = args.get("enabled").and_then(|v| v.as_bool()) {
                client::sync_state::set_enabled(enabled);
            }
            if let Some(n) = args
                .get("intervalMin")
                .or(args.get("interval_min"))
                .and_then(|v| v.as_u64())
            {
                client::sync_state::set_interval_min(n as u32);
            }
            client::save_setting("sync_enabled", json!(client::sync_state::is_enabled()));
            client::save_setting(
                "sync_interval_min",
                json!(client::sync_state::interval_min()),
            );
            serde_json::to_value(client::sync_state::snapshot()).map_err(|e| e.to_string())
        }

        "telegram_accounts_list" => {
            let pdd = host::data_dir();
            let list = client::accounts::list_profiles(&pdd);
            serde_json::to_value(list).map_err(|e| e.to_string())
        }

        "telegram_accounts_save_current" => {
            let label: String = args
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let phone: Option<String> = args
                .get("phone")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let user_id: Option<i64> = args
                .get("userId")
                .or(args.get("user_id"))
                .and_then(|v| v.as_i64());
            let pdd = host::data_dir();
            let p =
                client::accounts::save_current_as_profile(&pdd, &label, phone.as_deref(), user_id)
                    .map_err(|e| e.to_string())?;
            serde_json::to_value(p).map_err(|e| e.to_string())
        }

        "telegram_accounts_restore" => {
            let id: String = serde_json::from_value(args.get("id").cloned().ok_or("missing 'id'")?)
                .map_err(|e| e.to_string())?;
            let pdd = host::data_dir();
            client::accounts::restore_profile(&pdd, &id).map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true, "needs_restart": true }))
        }

        "telegram_accounts_remove" => {
            let id: String = serde_json::from_value(args.get("id").cloned().ok_or("missing 'id'")?)
                .map_err(|e| e.to_string())?;
            let pdd = host::data_dir();
            client::accounts::remove_profile(&pdd, &id).map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        }

        "telegram_accounts_rename" => {
            let id: String = serde_json::from_value(args.get("id").cloned().ok_or("missing 'id'")?)
                .map_err(|e| e.to_string())?;
            let label: String =
                serde_json::from_value(args.get("label").cloned().ok_or("missing 'label'")?)
                    .map_err(|e| e.to_string())?;
            let pdd = host::data_dir();
            let p =
                client::accounts::rename_profile(&pdd, &id, &label).map_err(|e| e.to_string())?;
            serde_json::to_value(p).map_err(|e| e.to_string())
        }

        "telegram_accounts_backup_now" => {
            let pdd = host::data_dir();
            let path = client::accounts::backup_current_session(&pdd).map_err(|e| e.to_string())?;
            Ok(json!({
                "path": path.to_string_lossy(),
                "name": path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
            }))
        }

        "telegram_accounts_list_backups" => {
            let pdd = host::data_dir();
            let list = client::accounts::list_backups(&pdd);
            let json: Vec<serde_json::Value> = list
                .into_iter()
                .map(|(name, mtime)| json!({ "name": name, "modified_at": mtime }))
                .collect();
            Ok(serde_json::Value::Array(json))
        }

        "telegram_bandwidth_stats" => {
            let pdd = host::data_dir();
            let s = client::bandwidth::load(&pdd);
            let pct = if s.quota_bytes > 0 {
                (s.used_today_bytes as f64 / s.quota_bytes as f64) * 100.0
            } else {
                0.0
            };
            Ok(json!({
                "used_today_bytes": s.used_today_bytes,
                "total_used_bytes": s.total_used_bytes,
                "quota_bytes": s.quota_bytes,
                "date": s.date,
                "percentage": pct.min(100.0),
            }))
        }

        "telegram_bandwidth_set_quota" => {
            let gb: u64 = serde_json::from_value(
                args.get("gb")
                    .or(args.get("quotaGb"))
                    .cloned()
                    .ok_or("missing 'gb'")?,
            )
            .map_err(|e| e.to_string())?;
            let pdd = host::data_dir();
            let s = client::bandwidth::set_quota_gb(&pdd, gb).map_err(|e| e.to_string())?;
            Ok(json!({
                "quota_bytes": s.quota_bytes,
                "used_today_bytes": s.used_today_bytes,
            }))
        }

        "telegram_bandwidth_reset" => {
            let pdd = host::data_dir();
            let s = client::bandwidth::reset_today(&pdd).map_err(|e| e.to_string())?;
            Ok(json!({
                "used_today_bytes": s.used_today_bytes,
                "date": s.date,
            }))
        }

        "telegram_perf_set" => {
            let max_threads: usize = serde_json::from_value(
                args.get("maxThreads")
                    .or(args.get("max_threads"))
                    .cloned()
                    .ok_or("missing 'maxThreads'")?,
            )
            .map_err(|e| e.to_string())?;
            let clamped = max_threads.clamp(1, 32);
            client::parallel_download::set_max_threads_cap(clamped);
            client::save_setting("max_threads", json!(clamped));
            Ok(json!({ "max_threads": clamped }))
        }

        // ---- new in the app -------------------------------------------------
        "telegram_resolve_links" => {
            let text: String =
                opt_string(&args, &["text", "url", "urls"]).ok_or("missing 'text'")?;
            let r = client::links::resolve_links(&session, &text, fix_ext)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }

        "telegram_download_links" => {
            let text: String =
                opt_string(&args, &["text", "url", "urls"]).ok_or("missing 'text'")?;
            let output_dir = opt_string(&args, &["output_dir", "outputDir"])
                .map(std::path::PathBuf::from)
                .unwrap_or_else(client::download_dir);
            let r = client::links::resolve_links(&session, &text, fix_ext)
                .await
                .map_err(|e| e.to_string())?;
            if r.items.is_empty() {
                let why = r
                    .errors
                    .first()
                    .map(|e| e.error.clone())
                    .unwrap_or_else(|| "No downloadable media in these links".into());
                return Err(why);
            }
            let title = if r.items.len() == 1 {
                r.items[0].file_name.clone()
            } else {
                format!("{} ({} files)", r.items[0].chat_title, r.items.len())
            };
            let count = r.items.len();
            let batch_id = match opt_string(&args, &["template"]) {
                Some(t) => {
                    let planned: Vec<client::batch::Planned> = r
                        .items
                        .iter()
                        .map(|li| {
                            let it = api::TelegramMediaItem {
                                message_id: li.message_id,
                                file_name: li.file_name.clone(),
                                file_size: li.file_size,
                                media_type: li.media_type.clone(),
                                date: li.date,
                                webpage: None,
                                caption: li.caption.clone(),
                                grouped_id: li.grouped_id,
                            };
                            client::batch::plan_item(
                                &output_dir,
                                Some(&t),
                                None,
                                li.chat_id,
                                &li.chat_type,
                                &li.chat_title,
                                &it,
                            )
                        })
                        .collect();
                    client::batch::start(
                        session.clone(),
                        active.clone(),
                        planned,
                        title,
                        output_dir.clone(),
                        concurrent,
                        true,
                        false,
                    )
                    .await
                    .0
                }
                None => {
                    client::links::download_items(
                        session.clone(),
                        active.clone(),
                        r.items,
                        output_dir.clone(),
                        concurrent,
                        title,
                    )
                    .await
                }
            };
            Ok(json!({
                "batch_id": batch_id,
                "count": count,
                "skipped": r.skipped,
                "errors": r.errors,
                "output_dir": output_dir.to_string_lossy(),
            }))
        }

        "telegram_export_chat" => {
            let chat_id: i64 = serde_json::from_value(
                args.get("chat_id")
                    .or(args.get("chatId"))
                    .cloned()
                    .ok_or("missing 'chatId'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_type: String = serde_json::from_value(
                args.get("chat_type")
                    .or(args.get("chatType"))
                    .cloned()
                    .ok_or("missing 'chatType'")?,
            )
            .map_err(|e| e.to_string())?;
            let chat_title: String =
                opt_string(&args, &["chat_title", "chatTitle"]).unwrap_or_default();
            let html = opt_string(&args, &["format"])
                .map(|f| f.eq_ignore_ascii_case("html"))
                .unwrap_or(false);
            let include_media = args
                .get("include_media")
                .or(args.get("includeMedia"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let output_dir = opt_string(&args, &["output_dir", "outputDir"])
                .map(std::path::PathBuf::from)
                .unwrap_or_else(client::download_dir);

            let export_id = events::next_id();
            let cancel = CancellationToken::new();
            active.lock().await.insert(
                export_id,
                (format!("tg-export:{}", export_id), cancel.clone()),
            );
            tokio::spawn(async move {
                let req = client::export::ExportRequest {
                    chat_id,
                    chat_type,
                    chat_title,
                    html,
                    include_media,
                    output_dir,
                    limit,
                };
                let res = Box::pin(client::export::run(&session, export_id, req, &cancel)).await;
                active.lock().await.remove(&export_id);
                let payload = match res {
                    Ok((path, count)) => {
                        json!({ "export_id": export_id, "success": true, "path": path.to_string_lossy(), "message_count": count, "error": null })
                    }
                    Err(e) => {
                        json!({ "export_id": export_id, "success": false, "path": null, "message_count": 0, "error": e.to_string() })
                    }
                };
                events::emit("telegram-export-complete", payload);
            });
            Ok(json!({ "export_id": export_id }))
        }

        "telegram_settings_get" => {
            let mut s = client::settings();
            if let Some(obj) = s.as_object_mut() {
                obj.entry("download_dir")
                    .or_insert(json!(client::download_dir().to_string_lossy()));
            }
            Ok(s)
        }

        "telegram_list_commands" => Ok(json!(COMMANDS)),

        // ---- player -----------------------------------------------------------
        "telegram_stream_url" => {
            let (chat_id, chat_type) = chat_of(&args)?;
            let message_id = message_id_of(&args)?;
            client::player::stream_url(&session, chat_id, &chat_type, message_id).await
        }

        "telegram_playback_get" => {
            let (chat_id, _) = chat_of(&args)?;
            let message_id = message_id_of(&args)?;
            Ok(client::player::playback_get(chat_id, message_id))
        }

        "telegram_playback_set" => {
            let (chat_id, chat_type) = chat_of(&args)?;
            let message_id = message_id_of(&args)?;
            let position = num_of(&args, &["positionSec", "position_sec", "position"])
                .ok_or("missing 'positionSec'")?;
            let duration =
                num_of(&args, &["durationSec", "duration_sec", "duration"]).unwrap_or(0.0);
            client::player::playback_set(
                chat_id,
                &chat_type,
                message_id,
                position,
                duration,
                opt_string(&args, &["fileName", "file_name"]),
                opt_string(&args, &["chatTitle", "chat_title"]),
            )
        }

        "telegram_playback_recent" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            serde_json::to_value(client::player::playback_recent(limit)).map_err(|e| e.to_string())
        }

        "telegram_open_external" => {
            let url = opt_string(&args, &["url"]).ok_or("missing 'url'")?;
            let player = opt_string(&args, &["player"]).unwrap_or_else(|| "default".into());
            client::player::open_external(&url, &player)?;
            Ok(json!({ "ok": true }))
        }

        "telegram_external_players" => Ok(json!(client::player::external_players())),

        // ---- auto-save --------------------------------------------------------
        "telegram_watch_list" => {
            serde_json::to_value(client::watch::list()).map_err(|e| e.to_string())
        }

        "telegram_watch_add" => {
            let rule = args.get("rule").cloned().unwrap_or_else(|| args.clone());
            let backfill = args
                .get("backfill")
                .or(rule.get("backfill"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .min(1_000) as usize;
            let r = client::watch::add(&session, &active, &rule, backfill).await?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }

        "telegram_watch_update" => {
            let id = opt_string(&args, &["id"]).ok_or("missing 'id'")?;
            let patch = args.get("patch").cloned().ok_or("missing 'patch'")?;
            let r = client::watch::update(&id, &patch)?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }

        "telegram_watch_remove" => {
            let id = opt_string(&args, &["id"]).ok_or("missing 'id'")?;
            Ok(json!({ "ok": client::watch::remove(&id) }))
        }

        "telegram_watch_run" => match opt_string(&args, &["id"]) {
            Some(id) => client::watch::run_rule(&session, &active, &id).await,
            None => Ok(json!(client::watch::run_all(&session, &active).await)),
        },

        "telegram_watch_settings" => {
            if let Some(n) = args
                .get("intervalMin")
                .or(args.get("interval_min"))
                .and_then(|v| v.as_u64())
            {
                client::watch::set_interval_min(n as u32);
            }
            Ok(json!({ "interval_min": client::watch::interval_min() }))
        }

        // ---- whole-chat download ------------------------------------------------
        "telegram_preview_chat_download" => {
            let plan = plan_chat_download(&session, &args, fix_ext).await?;
            let total_bytes: u64 = plan.planned.iter().map(|p| p.file_size).sum();
            let sample: Vec<serde_json::Value> = plan
                .items
                .iter()
                .zip(plan.planned.iter())
                .take(20)
                .map(|(it, p)| {
                    let mut v = serde_json::to_value(it).unwrap_or_else(|_| json!({}));
                    if let Some(obj) = v.as_object_mut() {
                        let rel = p.path.strip_prefix(&plan.root).unwrap_or(&p.path);
                        obj.insert("path".into(), json!(rel.to_string_lossy()));
                        obj.insert("exists".into(), json!(client::batch::already_there(p)));
                    }
                    v
                })
                .collect();
            let duplicates = if plan.dedupe {
                plan.planned
                    .iter()
                    .zip(plan.uids.iter())
                    .filter(|(p, uid)| {
                        !client::batch::already_there(p)
                            && client::dedupe::find(uid.as_deref(), p.file_size, &p.file_name)
                                .is_some_and(|found| found != p.path)
                    })
                    .count()
            } else {
                0
            };
            Ok(json!({
                "count": plan.planned.len(),
                "total_bytes": total_bytes,
                "scanned": plan.scanned,
                "output_dir": plan.root.to_string_lossy(),
                "sample": sample,
                "scan_id": plan.scan_id,
                "from_index": true,
                "duplicates": duplicates,
            }))
        }

        "telegram_download_chat" => {
            let plan = plan_chat_download(&session, &args, fix_ext).await?;
            let count = plan.planned.len();
            if count == 0 {
                return Ok(json!({
                    "batch_id": null,
                    "count": 0,
                    "scan_id": plan.scan_id,
                    "from_index": true,
                }));
            }
            let title = if plan.title.is_empty() {
                format!("{} files", count)
            } else {
                plan.title.clone()
            };
            let (batch_id, _) = client::batch::start(
                session.clone(),
                active.clone(),
                plan.planned,
                title,
                plan.root,
                concurrent,
                plan.skip_same,
                plan.dedupe,
            )
            .await;
            Ok(json!({
                "batch_id": batch_id,
                "count": count,
                "scan_id": plan.scan_id,
                "from_index": true,
            }))
        }

        // ---- upload -------------------------------------------------------------
        "telegram_upload_paths" => {
            let (chat_id, chat_type) = chat_of(&args)?;
            let paths: Vec<String> = match args.get("paths").or(args.get("path")) {
                Some(serde_json::Value::Array(a)) => a
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect(),
                Some(serde_json::Value::String(p)) => vec![p.clone()],
                _ => return Err("missing 'paths'".into()),
            };
            let files = client::upload::collect_files(&paths);
            if files.is_empty() {
                return Err("No files found in the chosen paths".into());
            }
            let count = files.len();
            let opts = client::upload::UploadOpts {
                caption: opt_string(&args, &["caption"]),
                as_document: bool_of(&args, &["asDocument", "as_document"]),
                delete_after: bool_of(&args, &["deleteAfter", "delete_after"]),
                group_albums: bool_of(&args, &["groupAlbums", "group_albums"]),
            };
            let batch_id = client::upload::start(
                session.clone(),
                active.clone(),
                chat_id,
                chat_type,
                files,
                opts,
            )
            .await?;
            Ok(json!({ "batch_id": batch_id, "count": count }))
        }
        // ---- real time -----------------------------------------------------------
        "telegram_realtime_get" => Ok(client::realtime::state()),

        "telegram_realtime_set" => {
            let on = opt_bool(&args, &["enabled"]).ok_or("missing 'enabled'")?;
            Ok(client::realtime::set_enabled(on))
        }

        // ---- history index ---------------------------------------------------------
        "telegram_scan_chat" => {
            let (chat_id, chat_type) = chat_of(&args)?;
            let scan_id = opt_string(&args, &["scanId", "scan_id"])
                .unwrap_or_else(client::index::new_scan_id);
            let max_scan = num_of(&args, &["maxScan", "max_scan"])
                .unwrap_or(5_000.0)
                .max(0.0) as usize;
            let cancel = client::index::register_scan(&scan_id);
            let opts = client::index::UpdateOpts {
                scan_id: &scan_id,
                max_scan,
                takeout: bool_of(&args, &["takeout"]),
                filter: None,
                stop_at_id: 0,
                emit: true,
            };
            let res =
                client::index::update(&session, chat_id, &chat_type, &opts, &cancel, |_, _| true)
                    .await;
            client::index::unregister_scan(&scan_id);
            let res = res.map_err(|e| e.to_string())?;
            let stats = client::index::stats(chat_id);
            Ok(json!({
                "scan_id": scan_id,
                "chat_id": chat_id,
                "scanned": res.scanned,
                "indexed": stats["indexed"],
                "newest_id": res.coverage.newest_id,
                "oldest_id": res.coverage.oldest_id,
                "complete": res.coverage.complete,
                "cancelled": res.cancelled,
            }))
        }

        "telegram_scan_cancel" => {
            let id = opt_string(&args, &["scanId", "scan_id"]).ok_or("missing 'scanId'")?;
            Ok(json!({ "cancelled": client::index::cancel_scan(&id) }))
        }

        "telegram_index_stats" => {
            let (chat_id, _) = chat_of(&args)?;
            Ok(client::index::stats(chat_id))
        }

        "telegram_index_clear" => {
            let chat = chat_of(&args).ok().map(|(id, _)| id);
            client::index::clear(chat);
            Ok(json!({ "ok": true }))
        }

        // ---- duplicates -------------------------------------------------------------
        "telegram_dedupe_stats" => Ok(client::dedupe::stats()),

        "telegram_dedupe_set" => {
            let on = opt_bool(&args, &["enabled"]).ok_or("missing 'enabled'")?;
            client::dedupe::set_enabled(on);
            Ok(client::dedupe::stats())
        }

        "telegram_dedupe_clear" => {
            client::dedupe::clear();
            Ok(client::dedupe::stats())
        }

        // ---- transcripts ------------------------------------------------------------
        "telegram_transcribe" => {
            let (chat_id, chat_type) = chat_of(&args)?;
            let message_id = message_id_of(&args)?;
            let lang = opt_string(&args, &["lang", "language"]);
            let model = opt_string(&args, &["model"]);
            let t = client::transcribe::transcribe(
                &session,
                chat_id,
                &chat_type,
                message_id,
                lang.as_deref(),
                model.as_deref(),
                bool_of(&args, &["force"]),
            )
            .await?;
            serde_json::to_value(t).map_err(|e| e.to_string())
        }

        "telegram_voice_transcribe_native" => {
            let (chat_id, chat_type) = chat_of(&args)?;
            let message_id = message_id_of(&args)?;
            client::transcribe::native(&session, chat_id, &chat_type, message_id).await
        }

        "telegram_transcript_get" => {
            let (chat_id, _) = chat_of(&args)?;
            let message_id = message_id_of(&args)?;
            serde_json::to_value(client::transcribe::cached(chat_id, message_id))
                .map_err(|e| e.to_string())
        }

        // ---- local Whisper (core::asr) ------------------------------------------------
        "telegram_asr_status" => {
            serde_json::to_value(crate::core::asr::status()).map_err(|e| e.to_string())
        }

        "telegram_asr_install" => {
            let model = opt_string(&args, &["model"]);
            let progress = client::transcribe::install_progress(model.as_deref());
            let st = crate::core::asr::install(model.as_deref(), &progress)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_value(st).map_err(|e| e.to_string())
        }

        // The app answers this one itself (it owns the /video library).
        "telegram_add_to_watch" => {
            Err("Adding to Watch is only available in the OmniGet app".into())
        }

        // ---- approvals of agent actions ----------------------------------------------
        "telegram_approval_list" => {
            serde_json::to_value(super::approvals::pending()).map_err(|e| e.to_string())
        }

        "telegram_approval_answer" => {
            let id = opt_string(&args, &["id"]).ok_or("missing 'id'")?;
            let approve = opt_bool(&args, &["approve", "approved"]).ok_or("missing 'approve'")?;
            super::approvals::answer(&id, approve)
        }

        _ => Err(format!("Unknown command: {}", command)),
    }
}
