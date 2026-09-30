use std::path::{Path, PathBuf};
use std::time::Duration;

use grammers_client::grammers_tl_types as tl;
use grammers_client::types::Peer;
use grammers_client::Client;
use grammers_tl_types::Serializable;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::auth::TelegramSessionHandle;

fn mime_to_ext(mime: &str) -> &'static str {
    match mime {
        // Video
        "video/mp4" => ".mp4",
        "video/x-matroska" => ".mkv",
        "video/webm" => ".webm",
        "video/quicktime" => ".mov",
        "video/x-msvideo" => ".avi",
        "video/mpeg" => ".mpeg",
        "video/3gpp" => ".3gp",
        "video/x-flv" => ".flv",
        // Audio
        "audio/mpeg" | "audio/mp3" => ".mp3",
        "audio/ogg" => ".ogg",
        "audio/x-opus+ogg" => ".opus",
        "audio/flac" | "audio/x-flac" => ".flac",
        "audio/x-wav" | "audio/wav" => ".wav",
        "audio/aac" | "audio/x-aac" => ".aac",
        "audio/mp4" | "audio/x-m4a" => ".m4a",
        "audio/x-ms-wma" => ".wma",
        // Image
        "image/jpeg" => ".jpg",
        "image/png" => ".png",
        "image/gif" => ".gif",
        "image/webp" => ".webp",
        "image/bmp" => ".bmp",
        "image/svg+xml" => ".svg",
        "image/tiff" => ".tiff",
        // Documents
        "application/pdf" => ".pdf",
        "application/zip" => ".zip",
        "application/x-rar-compressed" | "application/vnd.rar" => ".rar",
        "application/x-7z-compressed" => ".7z",
        "application/gzip" | "application/x-gzip" => ".gz",
        "application/x-tar" => ".tar",
        "application/json" => ".json",
        "application/xml" | "text/xml" => ".xml",
        "text/plain" => ".txt",
        "text/html" => ".html",
        "text/csv" => ".csv",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => ".docx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => ".xlsx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => ".pptx",
        "application/msword" => ".doc",
        "application/vnd.ms-excel" => ".xls",
        "application/vnd.ms-powerpoint" => ".ppt",
        "application/x-python-script" | "text/x-python" => ".py",
        "application/javascript" | "text/javascript" => ".js",
        // Fallback: try to extract from the subtype
        other => {
            if let Some(sub) = other.split('/').nth(1) {
                match sub {
                    "mp4" => ".mp4",
                    "mpeg" => ".mpeg",
                    "ogg" => ".ogg",
                    "webm" => ".webm",
                    "flac" => ".flac",
                    "wav" => ".wav",
                    "jpeg" => ".jpg",
                    "png" => ".png",
                    "gif" => ".gif",
                    _ => "",
                }
            } else {
                ""
            }
        }
    }
}

fn ensure_extension(name: &str, mime_type: &str) -> String {
    let path = Path::new(name);
    if let Some(ext) = path.extension() {
        let ext_str = ext.to_string_lossy().to_lowercase();
        let known = matches!(
            ext_str.as_str(),
            "mp4"
                | "mkv"
                | "webm"
                | "mov"
                | "avi"
                | "mpeg"
                | "3gp"
                | "flv"
                | "mp3"
                | "ogg"
                | "opus"
                | "flac"
                | "wav"
                | "aac"
                | "m4a"
                | "wma"
                | "jpg"
                | "jpeg"
                | "png"
                | "gif"
                | "webp"
                | "bmp"
                | "svg"
                | "tiff"
                | "pdf"
                | "zip"
                | "rar"
                | "7z"
                | "gz"
                | "tar"
                | "json"
                | "xml"
                | "txt"
                | "html"
                | "csv"
                | "doc"
                | "docx"
                | "xls"
                | "xlsx"
                | "ppt"
                | "pptx"
                | "py"
                | "js"
                | "ts"
                | "rs"
                | "go"
                | "c"
                | "cpp"
                | "h"
        );
        if known {
            return name.to_string();
        }
    }

    let ext = mime_to_ext(mime_type);
    if ext.is_empty() {
        name.to_string()
    } else {
        format!("{}{}", name, ext)
    }
}

pub fn parse_flood_wait(err: &str) -> Option<u64> {
    for pattern in &["FLOOD_WAIT_", "FLOOD_PREMIUM_WAIT_"] {
        if let Some(pos) = err.find(pattern) {
            let after = &err[pos + pattern.len()..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(secs) = digits.parse::<u64>() {
                return Some(secs);
            }
        }
    }
    None
}

pub fn parse_dc_migrate_error(msg: &str) -> Option<i32> {
    for pattern in &[
        "FILE_MIGRATE_",
        "PHONE_MIGRATE_",
        "USER_MIGRATE_",
        "NETWORK_MIGRATE_",
    ] {
        if let Some(pos) = msg.find(pattern) {
            let after = &msg[pos + pattern.len()..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(dc) = digits.parse::<i32>() {
                return Some(dc);
            }
        }
    }
    None
}

pub fn is_retryable_error(msg: &str) -> bool {
    if parse_flood_wait(msg).is_some() {
        return false;
    }
    let upper = msg.to_uppercase();
    if upper.contains("TIMEDOUT")
        || upper.contains("RPC_CALL_FAIL")
        || upper.contains("WORKER_BUSY_TOO_LONG_RETRY")
        || upper.contains("MEMORY LIMIT EXIT")
    {
        return true;
    }
    let lower = msg.to_lowercase();
    if lower.contains("incomplete download") {
        return true;
    }
    let network = [
        "connection reset",
        "timed out",
        "connection refused",
        "broken pipe",
        "unexpected eof",
        "internal server error",
        "temporarily unavailable",
        "transport error",
        "network",
        "rpc error",
    ];
    network.iter().any(|p| lower.contains(p))
}

fn retry_backoff_ms(attempt: u32) -> u64 {
    let base = 100u64;
    let factor = 1.1f64.powi(attempt as i32);
    let ms = (base as f64 * factor) as u64;
    ms.min(5000)
}

const RPC_TIMEOUT_SECS: u64 = 20;
const MAX_RETRY_ATTEMPTS: u32 = 3;

pub async fn invoke_with_flood_wait<R>(
    client: &Client,
    request: &R,
) -> Result<R::Return, grammers_mtsender::InvocationError>
where
    R: grammers_tl_types::RemoteCall + Serializable,
{
    for attempt in 0..MAX_RETRY_ATTEMPTS {
        let invoke_fut = client.invoke(request);
        let result =
            match tokio::time::timeout(Duration::from_secs(RPC_TIMEOUT_SECS), invoke_fut).await {
                Ok(r) => r,
                Err(_) => {
                    tracing::warn!(
                        "[tg-api] RPC timeout after {}s on attempt {}",
                        RPC_TIMEOUT_SECS,
                        attempt + 1
                    );
                    if attempt < MAX_RETRY_ATTEMPTS - 1 {
                        continue;
                    }
                    return Err(grammers_mtsender::InvocationError::Dropped);
                }
            };
        match result {
            Ok(r) => return Ok(r),
            Err(e) => {
                let err_str = e.to_string();
                if let Some(secs) = parse_flood_wait(&err_str) {
                    let wait = secs + 1;
                    tracing::warn!(
                        "[tg-api] FLOOD_WAIT_{} on attempt {}, waiting {}s",
                        secs,
                        attempt + 1,
                        wait
                    );
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                    continue;
                }
                if let Some(target_dc) = parse_dc_migrate_error(&err_str) {
                    tracing::info!(
                        "[tg-dc] FILE_MIGRATE_{} caught, retrying via dc={}",
                        target_dc,
                        target_dc
                    );
                    if let Err(auth_err) =
                        super::parallel_download::ensure_auth_on_dc(client, target_dc).await
                    {
                        tracing::warn!(
                            "[tg-dc] ensure_auth_on_dc({}) failed: {} — propagating original error",
                            target_dc,
                            auth_err
                        );
                        return Err(e);
                    }
                    tracing::info!("[tg-dc] using client for DC={}", target_dc);
                    let migrate_fut = client.invoke_in_dc(target_dc, request);
                    match tokio::time::timeout(Duration::from_secs(RPC_TIMEOUT_SECS), migrate_fut)
                        .await
                    {
                        Ok(Ok(r)) => return Ok(r),
                        Ok(Err(retry_err)) => {
                            tracing::warn!(
                                "[tg-dc] retry on dc={} failed: {}",
                                target_dc,
                                retry_err
                            );
                            return Err(retry_err);
                        }
                        Err(_) => {
                            tracing::warn!("[tg-dc] retry on dc={} timed out", target_dc);
                            return Err(grammers_mtsender::InvocationError::Dropped);
                        }
                    }
                }
                if is_retryable_error(&err_str) && attempt < MAX_RETRY_ATTEMPTS - 1 {
                    let backoff = retry_backoff_ms(attempt);
                    tracing::warn!(
                        "[tg-api] retryable error '{}' attempt={} backoff={}ms",
                        err_str.chars().take(120).collect::<String>(),
                        attempt + 1,
                        backoff
                    );
                    tokio::time::sleep(Duration::from_millis(backoff)).await;
                    continue;
                }
                return Err(e);
            }
        }
    }
    match tokio::time::timeout(
        Duration::from_secs(RPC_TIMEOUT_SECS),
        client.invoke(request),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => Err(grammers_mtsender::InvocationError::Dropped),
    }
}

#[cfg(test)]
mod dc_migrate_tests {
    use super::parse_dc_migrate_error;

    #[test]
    fn file_migrate_returns_dc() {
        assert_eq!(parse_dc_migrate_error("FILE_MIGRATE_5"), Some(5));
        assert_eq!(parse_dc_migrate_error("FILE_MIGRATE_2"), Some(2));
    }

    #[test]
    fn phone_migrate_returns_dc() {
        assert_eq!(parse_dc_migrate_error("PHONE_MIGRATE_4"), Some(4));
    }

    #[test]
    fn user_migrate_returns_dc() {
        assert_eq!(parse_dc_migrate_error("USER_MIGRATE_3"), Some(3));
    }

    #[test]
    fn network_migrate_returns_dc() {
        assert_eq!(parse_dc_migrate_error("NETWORK_MIGRATE_1"), Some(1));
    }

    #[test]
    fn migrate_inside_longer_message_is_extracted() {
        assert_eq!(
            parse_dc_migrate_error("RpcError { code: 303, name: \"FILE_MIGRATE_2\" }"),
            Some(2)
        );
    }

    #[test]
    fn no_migrate_returns_none() {
        assert_eq!(parse_dc_migrate_error("FLOOD_WAIT_30"), None);
        assert_eq!(parse_dc_migrate_error("MESSAGE_ID_INVALID"), None);
        assert_eq!(parse_dc_migrate_error(""), None);
        assert_eq!(parse_dc_migrate_error("random network error"), None);
    }

    #[test]
    fn malformed_migrate_returns_none() {
        assert_eq!(parse_dc_migrate_error("FILE_MIGRATE_"), None);
        assert_eq!(parse_dc_migrate_error("FILE_MIGRATE_abc"), None);
    }
}

#[cfg(test)]
mod retryable_tests {
    use super::is_retryable_error;

    #[test]
    fn timedout_is_retryable() {
        assert!(is_retryable_error("RPC error: Timedout"));
        assert!(is_retryable_error("network: TIMEDOUT after 20s"));
    }

    #[test]
    fn rpc_call_fail_is_retryable() {
        assert!(is_retryable_error("server returned RPC_CALL_FAIL"));
    }

    #[test]
    fn worker_busy_is_retryable() {
        assert!(is_retryable_error("WORKER_BUSY_TOO_LONG_RETRY"));
    }

    #[test]
    fn memory_limit_is_retryable() {
        assert!(is_retryable_error("memory limit exit"));
        assert!(is_retryable_error("MEMORY LIMIT EXIT"));
    }

    #[test]
    fn flood_wait_is_not_classified_as_retryable() {
        assert!(!is_retryable_error("FLOOD_WAIT_30"));
        assert!(!is_retryable_error("FLOOD_PREMIUM_WAIT_5"));
    }

    #[test]
    fn generic_errors_are_not_retryable() {
        assert!(!is_retryable_error("MESSAGE_ID_INVALID"));
        assert!(!is_retryable_error("CHANNEL_INVALID"));
        assert!(!is_retryable_error("auth required"));
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TelegramChat {
    pub id: i64,
    pub title: String,
    pub chat_type: String,
    #[serde(default)]
    pub peer_hash: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_date: Option<i32>,
    #[serde(default)]
    pub unread_count: i32,
    #[serde(default)]
    pub is_muted: bool,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub is_verified: bool,
    #[serde(default)]
    pub is_online: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TelegramSelf {
    pub user_id: i64,
    pub username: Option<String>,
    pub first_name: String,
    pub last_name: Option<String>,
    pub phone: Option<String>,
}

pub async fn delete_channel(handle: &TelegramSessionHandle, chat_id: i64) -> anyhow::Result<()> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let req = tl::functions::channels::DeleteChannel {
        channel: tl::enums::InputChannel::Channel(tl::types::InputChannel {
            channel_id: chat_id,
            access_hash,
        }),
    };
    client
        .invoke(&req)
        .await
        .map_err(|e| anyhow::anyhow!("channels.deleteChannel: {}", e))?;
    Ok(())
}

pub async fn leave_channel(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
) -> anyhow::Result<()> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    if chat_type == "channel" {
        let req = tl::functions::channels::LeaveChannel {
            channel: tl::enums::InputChannel::Channel(tl::types::InputChannel {
                channel_id: chat_id,
                access_hash,
            }),
        };
        client
            .invoke(&req)
            .await
            .map_err(|e| anyhow::anyhow!("channels.leaveChannel: {}", e))?;
    } else if chat_type == "group" {
        let req = tl::functions::messages::DeleteChatUser {
            revoke_history: false,
            chat_id,
            user_id: tl::enums::InputUser::UserSelf,
        };
        client
            .invoke(&req)
            .await
            .map_err(|e| anyhow::anyhow!("messages.deleteChatUser: {}", e))?;
    } else {
        return Err(anyhow::anyhow!(
            "leave_channel: chat_type '{}' not supported (use channel or group)",
            chat_type
        ));
    }
    Ok(())
}

pub async fn delete_history(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    just_clear: bool,
    revoke: bool,
) -> anyhow::Result<()> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let peer = make_input_peer(chat_id, chat_type, access_hash);
    let req = tl::functions::messages::DeleteHistory {
        just_clear,
        revoke,
        peer,
        max_id: 0,
        min_date: None,
        max_date: None,
    };
    client
        .invoke(&req)
        .await
        .map_err(|e| anyhow::anyhow!("messages.deleteHistory: {}", e))?;
    Ok(())
}

#[derive(Debug, Serialize, Clone)]
pub struct TelegramParticipant {
    pub user_id: i64,
    pub first_name: String,
    pub last_name: String,
    pub username: Option<String>,
    pub is_bot: bool,
    pub role: String,
    pub joined_at: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct TelegramParticipantsPage {
    pub count: i32,
    pub users: Vec<TelegramParticipant>,
}

pub async fn list_participants(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    filter: &str,
    offset: i32,
    limit: i32,
    search: Option<String>,
) -> anyhow::Result<TelegramParticipantsPage> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let q = search.unwrap_or_default();
    let participant_filter = match filter {
        "admins" => tl::enums::ChannelParticipantsFilter::ChannelParticipantsAdmins,
        "bots" => tl::enums::ChannelParticipantsFilter::ChannelParticipantsBots,
        "banned" => tl::enums::ChannelParticipantsFilter::ChannelParticipantsKicked(
            tl::types::ChannelParticipantsKicked { q: q.clone() },
        ),
        "restricted" => tl::enums::ChannelParticipantsFilter::ChannelParticipantsBanned(
            tl::types::ChannelParticipantsBanned { q: q.clone() },
        ),
        "search" => tl::enums::ChannelParticipantsFilter::ChannelParticipantsSearch(
            tl::types::ChannelParticipantsSearch { q: q.clone() },
        ),
        _ => tl::enums::ChannelParticipantsFilter::ChannelParticipantsRecent,
    };

    let req = tl::functions::channels::GetParticipants {
        channel: tl::enums::InputChannel::Channel(tl::types::InputChannel {
            channel_id: chat_id,
            access_hash,
        }),
        filter: participant_filter,
        offset,
        limit,
        hash: 0,
    };
    let resp = invoke_with_flood_wait(&client, &req)
        .await
        .map_err(|e| anyhow::anyhow!("channels.getParticipants: {}", e))?;

    let (count, participants_raw, users_raw) = match resp {
        tl::enums::channels::ChannelParticipants::Participants(p) => {
            (p.count, p.participants, p.users)
        }
        tl::enums::channels::ChannelParticipants::NotModified => (0, Vec::new(), Vec::new()),
    };

    let mut user_map: std::collections::HashMap<i64, &tl::types::User> =
        std::collections::HashMap::new();
    for u in &users_raw {
        if let tl::enums::User::User(uu) = u {
            user_map.insert(uu.id, uu);
        }
    }

    let mut users = Vec::with_capacity(participants_raw.len());
    for p in participants_raw {
        let (uid, role, joined_at): (i64, &str, Option<i32>) = match p {
            tl::enums::ChannelParticipant::Creator(c) => (c.user_id, "creator", None),
            tl::enums::ChannelParticipant::Admin(a) => (a.user_id, "admin", Some(a.date)),
            tl::enums::ChannelParticipant::Participant(c) => (c.user_id, "member", Some(c.date)),
            tl::enums::ChannelParticipant::Banned(b) => {
                let uid_extracted = match b.peer {
                    tl::enums::Peer::User(p) => p.user_id,
                    _ => 0,
                };
                (uid_extracted, "banned", Some(b.date))
            }
            tl::enums::ChannelParticipant::Left(l) => {
                let uid_extracted = match l.peer {
                    tl::enums::Peer::User(p) => p.user_id,
                    _ => 0,
                };
                (uid_extracted, "left", None)
            }
            tl::enums::ChannelParticipant::ParticipantSelf(s) => (s.user_id, "self", Some(s.date)),
        };
        if let Some(u) = user_map.get(&uid) {
            users.push(TelegramParticipant {
                user_id: uid,
                first_name: u.first_name.clone().unwrap_or_default(),
                last_name: u.last_name.clone().unwrap_or_default(),
                username: u.username.clone(),
                is_bot: u.bot,
                role: role.to_string(),
                joined_at,
            });
        } else {
            users.push(TelegramParticipant {
                user_id: uid,
                first_name: String::new(),
                last_name: String::new(),
                username: None,
                is_bot: false,
                role: role.to_string(),
                joined_at,
            });
        }
    }

    Ok(TelegramParticipantsPage { count, users })
}

pub async fn set_blocked(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    blocked: bool,
) -> anyhow::Result<()> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let peer = make_input_peer(chat_id, chat_type, access_hash);
    if blocked {
        let req = tl::functions::contacts::Block {
            my_stories_from: false,
            id: peer,
        };
        client
            .invoke(&req)
            .await
            .map_err(|e| anyhow::anyhow!("contacts.block: {}", e))?;
    } else {
        let req = tl::functions::contacts::Unblock {
            my_stories_from: false,
            id: peer,
        };
        client
            .invoke(&req)
            .await
            .map_err(|e| anyhow::anyhow!("contacts.unblock: {}", e))?;
    }
    Ok(())
}

pub async fn report_peer(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_ids: Vec<i32>,
    option: Vec<u8>,
    message: String,
) -> anyhow::Result<()> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let peer = make_input_peer(chat_id, chat_type, access_hash);
    let req = tl::functions::messages::Report {
        peer,
        id: message_ids,
        option,
        message,
    };
    client
        .invoke(&req)
        .await
        .map_err(|e| anyhow::anyhow!("messages.report: {}", e))?;
    Ok(())
}

pub async fn rename_channel(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    new_title: String,
) -> anyhow::Result<()> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let req = tl::functions::channels::EditTitle {
        channel: tl::enums::InputChannel::Channel(tl::types::InputChannel {
            channel_id: chat_id,
            access_hash,
        }),
        title: new_title,
    };
    client
        .invoke(&req)
        .await
        .map_err(|e| anyhow::anyhow!("channels.editTitle: {}", e))?;
    Ok(())
}

pub async fn create_folder_channel(
    handle: &TelegramSessionHandle,
    name: &str,
) -> anyhow::Result<TelegramChat> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    drop(guard);

    let title = format!("{} [og]", name.trim());
    let req = tl::functions::channels::CreateChannel {
        broadcast: true,
        megagroup: false,
        for_import: false,
        forum: false,
        title: title.clone(),
        about: "OmniGet Drive Folder\n[omniget-drive-folder]".to_string(),
        geo_point: None,
        address: None,
        ttl_period: None,
    };

    let updates = client
        .invoke(&req)
        .await
        .map_err(|e| anyhow::anyhow!("channels.createChannel: {}", e))?;

    let (channel_id, access_hash) = match &updates {
        tl::enums::Updates::Updates(u) => extract_channel_from_chats(&u.chats),
        tl::enums::Updates::Combined(u) => extract_channel_from_chats(&u.chats),
        _ => None,
    }
    .ok_or_else(|| anyhow::anyhow!("could not find channel id in response"))?;

    let _ = client
        .invoke(&tl::functions::messages::SetHistoryTtl {
            peer: tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                channel_id,
                access_hash,
            }),
            period: 0,
        })
        .await;

    {
        let mut g = handle.lock().await;
        g.peer_hashes.insert(channel_id, access_hash);
    }

    Ok(TelegramChat {
        id: channel_id,
        title,
        chat_type: "channel".to_string(),
        peer_hash: access_hash,
        last_message: None,
        last_message_date: None,
        unread_count: 0,
        is_muted: false,
        is_pinned: false,
        is_verified: false,
        is_online: false,
    })
}

pub fn extract_channel_from_chats_pub(chats: &[tl::enums::Chat]) -> Option<(i64, i64)> {
    extract_channel_from_chats(chats)
}

fn extract_channel_from_chats(chats: &[tl::enums::Chat]) -> Option<(i64, i64)> {
    for c in chats {
        if let tl::enums::Chat::Channel(ch) = c {
            return Some((ch.id, ch.access_hash.unwrap_or(0)));
        }
    }
    None
}

pub async fn upload_media(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    file_path: &Path,
    caption: Option<&str>,
) -> anyhow::Result<i32> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let input_peer = make_input_peer(chat_id, chat_type, access_hash);
    let uploaded = client
        .upload_file(file_path)
        .await
        .map_err(|e| anyhow::anyhow!("upload_file: {}", e))?;

    let mut msg = grammers_client::InputMessage::default();
    if let Some(c) = caption {
        if !c.is_empty() {
            msg = msg.text(c.to_string());
        }
    }
    let msg = msg.file(uploaded);

    let sent = client
        .send_message(input_peer, msg)
        .await
        .map_err(|e| anyhow::anyhow!("send_message: {}", e))?;
    Ok(sent.id())
}

pub async fn delete_messages(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_ids: Vec<i32>,
    revoke: bool,
) -> anyhow::Result<u32> {
    if message_ids.is_empty() {
        return Ok(0);
    }
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let is_channel = chat_type == "channel" || (chat_type == "group" && access_hash != 0);

    if is_channel {
        let req = tl::functions::channels::DeleteMessages {
            channel: tl::enums::InputChannel::Channel(tl::types::InputChannel {
                channel_id: chat_id,
                access_hash,
            }),
            id: message_ids,
        };
        let res = client
            .invoke(&req)
            .await
            .map_err(|e| anyhow::anyhow!("channels.deleteMessages: {}", e))?;
        let tl::enums::messages::AffectedMessages::Messages(a) = res;
        Ok(a.pts as u32)
    } else {
        let req = tl::functions::messages::DeleteMessages {
            revoke,
            id: message_ids,
        };
        let res = client
            .invoke(&req)
            .await
            .map_err(|e| anyhow::anyhow!("messages.deleteMessages: {}", e))?;
        let tl::enums::messages::AffectedMessages::Messages(a) = res;
        Ok(a.pts as u32)
    }
}

#[allow(clippy::too_many_arguments)] // mirrors the command arguments one to one
pub async fn forward_messages(
    handle: &TelegramSessionHandle,
    from_chat_id: i64,
    from_chat_type: &str,
    to_chat_id: i64,
    to_chat_type: &str,
    message_ids: Vec<i32>,
    drop_author: bool,
    drop_captions: bool,
) -> anyhow::Result<()> {
    if message_ids.is_empty() {
        return Ok(());
    }
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let from_hash = guard.peer_hashes.get(&from_chat_id).copied().unwrap_or(0);
    let to_hash = guard.peer_hashes.get(&to_chat_id).copied().unwrap_or(0);
    drop(guard);

    let from_peer = make_input_peer(from_chat_id, from_chat_type, from_hash);
    let to_peer = make_input_peer(to_chat_id, to_chat_type, to_hash);

    let mut random_ids = Vec::with_capacity(message_ids.len());
    for _ in 0..message_ids.len() {
        random_ids.push(rand::random::<i64>());
    }

    let req = tl::functions::messages::ForwardMessages {
        silent: false,
        background: false,
        with_my_score: false,
        drop_author,
        drop_media_captions: drop_captions,
        noforwards: false,
        allow_paid_floodskip: false,
        allow_paid_stars: None,
        from_peer,
        id: message_ids,
        random_id: random_ids,
        to_peer,
        top_msg_id: None,
        reply_to: None,
        schedule_date: None,
        send_as: None,
        quick_reply_shortcut: None,
        video_timestamp: None,
        suggested_post: None,
    };
    client
        .invoke(&req)
        .await
        .map_err(|e| anyhow::anyhow!("messages.forwardMessages: {}", e))?;
    Ok(())
}

pub async fn edit_caption(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
    caption: String,
) -> anyhow::Result<()> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let peer = make_input_peer(chat_id, chat_type, access_hash);
    let req = tl::functions::messages::EditMessage {
        no_webpage: false,
        invert_media: false,
        peer,
        id: message_id,
        message: Some(caption),
        media: None,
        reply_markup: None,
        entities: None,
        schedule_date: None,
        quick_reply_shortcut_id: None,
    };
    client
        .invoke(&req)
        .await
        .map_err(|e| anyhow::anyhow!("messages.editMessage: {}", e))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct TelegramFullChannelInfo {
    pub chat_id: i64,
    pub title: String,
    pub about: String,
    pub username: Option<String>,
    pub participants_count: Option<i32>,
}

pub async fn full_channel_info(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
) -> anyhow::Result<TelegramFullChannelInfo> {
    if chat_type != "channel" && chat_type != "group" {
        return Err(anyhow::anyhow!(
            "full_channel_info: only channel/group supported, got '{}'",
            chat_type
        ));
    }
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let req = tl::functions::channels::GetFullChannel {
        channel: tl::enums::InputChannel::Channel(tl::types::InputChannel {
            channel_id: chat_id,
            access_hash,
        }),
    };
    let resp = invoke_with_flood_wait(&client, &req)
        .await
        .map_err(|e| anyhow::anyhow!("channels.GetFullChannel: {}", e))?;
    let tl::enums::messages::ChatFull::Full(messages_chat_full) = resp;
    let full_chat = messages_chat_full.full_chat;
    let chats = messages_chat_full.chats;

    let (title, username, participants_count) = match &full_chat {
        tl::enums::ChatFull::ChannelFull(cf) => {
            let title_str = chats
                .iter()
                .find_map(|c| match c {
                    tl::enums::Chat::Channel(ch) if ch.id == chat_id => Some(ch.title.clone()),
                    tl::enums::Chat::Chat(ch) if ch.id == chat_id => Some(ch.title.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let uname = chats.iter().find_map(|c| match c {
                tl::enums::Chat::Channel(ch) if ch.id == chat_id => ch.username.clone(),
                _ => None,
            });
            (title_str, uname, cf.participants_count)
        }
        tl::enums::ChatFull::Full(_) => {
            let title_str = chats
                .iter()
                .find_map(|c| match c {
                    tl::enums::Chat::Chat(ch) if ch.id == chat_id => Some(ch.title.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            (title_str, None, None)
        }
    };
    let about = match &full_chat {
        tl::enums::ChatFull::ChannelFull(cf) => cf.about.clone(),
        tl::enums::ChatFull::Full(f) => f.about.clone(),
    };

    Ok(TelegramFullChannelInfo {
        chat_id,
        title,
        about,
        username,
        participants_count,
    })
}

pub async fn set_mute(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    mute_until: i64,
) -> anyhow::Result<()> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);
    let input_peer = make_input_peer(chat_id, chat_type, access_hash);
    let mute_clamped = mute_until.clamp(0, i32::MAX as i64) as i32;
    let req = tl::functions::account::UpdateNotifySettings {
        peer: tl::enums::InputNotifyPeer::Peer(tl::types::InputNotifyPeer { peer: input_peer }),
        settings: tl::enums::InputPeerNotifySettings::Settings(
            tl::types::InputPeerNotifySettings {
                show_previews: None,
                silent: None,
                mute_until: Some(mute_clamped),
                sound: None,
                stories_muted: None,
                stories_hide_sender: None,
                stories_sound: None,
            },
        ),
    };
    let _ = invoke_with_flood_wait(&client, &req)
        .await
        .map_err(|e| anyhow::anyhow!("account.UpdateNotifySettings: {}", e))?;
    tracing::info!(
        "[tg-api] set_mute chat={} mute_until={}",
        chat_id,
        mute_clamped
    );
    Ok(())
}

pub async fn toggle_pin(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    pinned: bool,
) -> anyhow::Result<()> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);
    let input_peer = make_input_peer(chat_id, chat_type, access_hash);
    let req = tl::functions::messages::ToggleDialogPin {
        pinned,
        peer: tl::enums::InputDialogPeer::Peer(tl::types::InputDialogPeer { peer: input_peer }),
    };
    let _ = invoke_with_flood_wait(&client, &req)
        .await
        .map_err(|e| anyhow::anyhow!("messages.ToggleDialogPin: {}", e))?;
    tracing::info!("[tg-api] toggle_pin chat={} pinned={}", chat_id, pinned);
    Ok(())
}

pub async fn set_archived(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    archived: bool,
) -> anyhow::Result<()> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);
    let input_peer = make_input_peer(chat_id, chat_type, access_hash);
    let folder_id = if archived { 1 } else { 0 };
    let req = tl::functions::folders::EditPeerFolders {
        folder_peers: vec![tl::enums::InputFolderPeer::Peer(
            tl::types::InputFolderPeer {
                peer: input_peer,
                folder_id,
            },
        )],
    };
    let _ = invoke_with_flood_wait(&client, &req)
        .await
        .map_err(|e| anyhow::anyhow!("folders.EditPeerFolders: {}", e))?;
    tracing::info!(
        "[tg-api] set_archived chat={} archived={}",
        chat_id,
        archived
    );
    Ok(())
}

pub async fn reorder_pinned(
    handle: &TelegramSessionHandle,
    items: Vec<(i64, String)>,
) -> anyhow::Result<()> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let mut order: Vec<tl::enums::InputDialogPeer> = Vec::with_capacity(items.len());
    for (chat_id, chat_type) in &items {
        let access_hash = guard.peer_hashes.get(chat_id).copied().unwrap_or(0);
        let input_peer = make_input_peer(*chat_id, chat_type, access_hash);
        order.push(tl::enums::InputDialogPeer::Peer(
            tl::types::InputDialogPeer { peer: input_peer },
        ));
    }
    drop(guard);
    let req = tl::functions::messages::ReorderPinnedDialogs {
        force: true,
        folder_id: 0,
        order,
    };
    let _ = invoke_with_flood_wait(&client, &req)
        .await
        .map_err(|e| anyhow::anyhow!("messages.ReorderPinnedDialogs: {}", e))?;
    tracing::info!("[tg-api] reorder_pinned with {} items", items.len());
    Ok(())
}

pub async fn init_takeout(handle: &TelegramSessionHandle) -> anyhow::Result<i64> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    if let Some(existing) = guard.takeout_session_id {
        drop(guard);
        tracing::info!("[tg-takeout] reusing existing session id={}", existing);
        return Ok(existing);
    }
    drop(guard);

    let req = tl::functions::account::InitTakeoutSession {
        contacts: false,
        message_users: false,
        message_chats: true,
        message_megagroups: true,
        message_channels: true,
        files: true,
        file_max_size: Some(1_500_000_000),
    };
    let resp = invoke_with_flood_wait(&client, &req)
        .await
        .map_err(|e| anyhow::anyhow!("account.InitTakeoutSession: {}", e))?;
    let id = match resp {
        tl::enums::account::Takeout::Takeout(t) => t.id,
    };
    let mut g = handle.lock().await;
    g.takeout_session_id = Some(id);
    drop(g);
    tracing::info!("[tg-takeout] init session id={}", id);
    Ok(id)
}

pub async fn finish_takeout(handle: &TelegramSessionHandle, success: bool) -> anyhow::Result<()> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let id = guard.takeout_session_id;
    drop(guard);
    if id.is_none() {
        return Ok(());
    }
    let req = tl::functions::account::FinishTakeoutSession { success };
    let _ = invoke_with_flood_wait(&client, &req)
        .await
        .map_err(|e| anyhow::anyhow!("account.FinishTakeoutSession: {}", e))?;
    let mut g = handle.lock().await;
    g.takeout_session_id = None;
    drop(g);
    tracing::info!("[tg-takeout] finished session success={}", success);
    Ok(())
}

pub async fn invoke_with_takeout<R>(
    client: &Client,
    takeout_id: i64,
    request: R,
) -> Result<R::Return, grammers_mtsender::InvocationError>
where
    R: grammers_tl_types::RemoteCall + Serializable,
    R::Return: grammers_tl_types::Deserializable,
{
    let wrapped = tl::functions::InvokeWithTakeout {
        takeout_id,
        query: request,
    };
    invoke_with_flood_wait(client, &wrapped).await
}

pub async fn search_global(
    handle: &TelegramSessionHandle,
    query: &str,
    limit: u32,
    fix_extensions: bool,
) -> anyhow::Result<Vec<TelegramMediaItem>> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    drop(guard);

    let req = tl::functions::messages::SearchGlobal {
        broadcasts_only: false,
        groups_only: false,
        users_only: false,
        folder_id: None,
        q: query.to_string(),
        filter: tl::enums::MessagesFilter::InputMessagesFilterEmpty,
        min_date: 0,
        max_date: 0,
        offset_rate: 0,
        offset_peer: tl::enums::InputPeer::Empty,
        offset_id: 0,
        limit: limit as i32,
    };
    let result = client
        .invoke(&req)
        .await
        .map_err(|e| anyhow::anyhow!("messages.searchGlobal: {}", e))?;
    let messages = match result {
        tl::enums::messages::Messages::Messages(m) => m.messages,
        tl::enums::messages::Messages::Slice(m) => m.messages,
        tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
        tl::enums::messages::Messages::NotModified(_) => vec![],
    };
    let mut items = Vec::new();
    for raw_msg in messages {
        let msg = match raw_msg {
            tl::enums::Message::Message(m) => m,
            _ => continue,
        };
        let caption_text = if msg.message.trim().is_empty() {
            None
        } else {
            Some(msg.message.clone())
        };
        let grouped = msg.grouped_id;
        if let Some(raw_media) = msg.media {
            if let Some(wp) = extract_webpage_info_raw(&raw_media) {
                let display = wp
                    .title
                    .clone()
                    .or_else(|| wp.site_name.clone())
                    .unwrap_or_else(|| wp.url.clone());
                items.push(TelegramMediaItem {
                    message_id: msg.id,
                    file_name: display,
                    file_size: 0,
                    media_type: "webpage".to_string(),
                    date: msg.date as i64,
                    webpage: Some(wp),
                    caption: caption_text,
                    grouped_id: grouped,
                });
                continue;
            }
            if let Some((file_name, file_size, media_type_str)) =
                extract_raw_media_info(&raw_media, fix_extensions)
            {
                items.push(TelegramMediaItem {
                    message_id: msg.id,
                    file_name,
                    file_size,
                    media_type: media_type_str,
                    date: msg.date as i64,
                    webpage: None,
                    caption: caption_text,
                    grouped_id: grouped,
                });
            }
        }
    }
    Ok(items)
}

pub async fn search_global_hits(
    handle: &TelegramSessionHandle,
    query: &str,
    limit: u32,
    fix_extensions: bool,
) -> anyhow::Result<Vec<TelegramGlobalSearchHit>> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    drop(guard);

    let req = tl::functions::messages::SearchGlobal {
        broadcasts_only: false,
        groups_only: false,
        users_only: false,
        folder_id: None,
        q: query.to_string(),
        filter: tl::enums::MessagesFilter::InputMessagesFilterEmpty,
        min_date: 0,
        max_date: 0,
        offset_rate: 0,
        offset_peer: tl::enums::InputPeer::Empty,
        offset_id: 0,
        limit: limit as i32,
    };
    let result = client
        .invoke(&req)
        .await
        .map_err(|e| anyhow::anyhow!("messages.searchGlobal: {}", e))?;

    let (messages, chats_raw, users_raw) = match result {
        tl::enums::messages::Messages::Messages(m) => (m.messages, m.chats, m.users),
        tl::enums::messages::Messages::Slice(m) => (m.messages, m.chats, m.users),
        tl::enums::messages::Messages::ChannelMessages(m) => (m.messages, m.chats, m.users),
        tl::enums::messages::Messages::NotModified(_) => (vec![], vec![], vec![]),
    };

    let mut chat_map: std::collections::HashMap<i64, (String, String)> =
        std::collections::HashMap::new();
    for c in &chats_raw {
        match c {
            tl::enums::Chat::Channel(ch) => {
                let kind = if ch.megagroup { "group" } else { "channel" };
                chat_map.insert(ch.id, (ch.title.clone(), kind.to_string()));
            }
            tl::enums::Chat::Chat(ch) => {
                chat_map.insert(ch.id, (ch.title.clone(), "group".to_string()));
            }
            _ => {}
        }
    }
    let mut user_map: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    for u in &users_raw {
        if let tl::enums::User::User(uu) = u {
            let display = match (&uu.first_name, &uu.last_name) {
                (Some(f), Some(l)) => format!("{} {}", f, l).trim().to_string(),
                (Some(f), None) => f.clone(),
                (None, Some(l)) => l.clone(),
                (None, None) => uu
                    .username
                    .clone()
                    .map(|s| format!("@{}", s))
                    .unwrap_or_else(|| format!("Usuário {}", uu.id)),
            };
            user_map.insert(uu.id, display);
        }
    }

    let mut hits = Vec::new();
    for raw_msg in messages {
        let msg = match raw_msg {
            tl::enums::Message::Message(m) => m,
            _ => continue,
        };

        let (chat_id, chat_title, chat_type) = match &msg.peer_id {
            tl::enums::Peer::Channel(p) => {
                let id = p.channel_id;
                let (title, ct) = chat_map
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| (format!("Channel {}", id), "channel".to_string()));
                (id, title, ct)
            }
            tl::enums::Peer::Chat(p) => {
                let id = p.chat_id;
                let (title, ct) = chat_map
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| (format!("Grupo {}", id), "group".to_string()));
                (id, title, ct)
            }
            tl::enums::Peer::User(p) => {
                let id = p.user_id;
                let title = user_map
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| format!("Usuário {}", id));
                (id, title, "private".to_string())
            }
        };

        let caption_text = if msg.message.trim().is_empty() {
            None
        } else {
            Some(msg.message.clone())
        };
        let grouped = msg.grouped_id;

        if let Some(raw_media) = msg.media {
            if let Some(wp) = extract_webpage_info_raw(&raw_media) {
                let display = wp
                    .title
                    .clone()
                    .or_else(|| wp.site_name.clone())
                    .unwrap_or_else(|| wp.url.clone());
                hits.push(TelegramGlobalSearchHit {
                    chat_id,
                    chat_type: chat_type.clone(),
                    chat_title: chat_title.clone(),
                    item: TelegramMediaItem {
                        message_id: msg.id,
                        file_name: display,
                        file_size: 0,
                        media_type: "webpage".to_string(),
                        date: msg.date as i64,
                        webpage: Some(wp),
                        caption: caption_text,
                        grouped_id: grouped,
                    },
                });
                continue;
            }
            if let Some((file_name, file_size, media_type_str)) =
                extract_raw_media_info(&raw_media, fix_extensions)
            {
                hits.push(TelegramGlobalSearchHit {
                    chat_id,
                    chat_type: chat_type.clone(),
                    chat_title: chat_title.clone(),
                    item: TelegramMediaItem {
                        message_id: msg.id,
                        file_name,
                        file_size,
                        media_type: media_type_str,
                        date: msg.date as i64,
                        webpage: None,
                        caption: caption_text,
                        grouped_id: grouped,
                    },
                });
            }
        }
    }
    Ok(hits)
}

pub async fn get_self(handle: &TelegramSessionHandle) -> anyhow::Result<TelegramSelf> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    drop(guard);

    let me = client
        .get_me()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(TelegramSelf {
        user_id: me.bare_id(),
        username: me.username().map(|s| s.to_string()),
        first_name: me.first_name().unwrap_or("").to_string(),
        last_name: me.last_name().map(|s| s.to_string()),
        phone: me.phone().filter(|p| !p.is_empty()).map(|s| s.to_string()),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct TelegramGlobalSearchHit {
    pub chat_id: i64,
    pub chat_type: String,
    pub chat_title: String,
    #[serde(flatten)]
    pub item: TelegramMediaItem,
}

#[derive(Debug, Clone, Serialize)]
pub struct TelegramMediaItem {
    pub message_id: i32,
    pub file_name: String,
    pub file_size: u64,
    pub media_type: String,
    pub date: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webpage: Option<TelegramWebpageInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouped_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TelegramWebpageInfo {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_sec: Option<i32>,
}

pub fn extract_webpage_info_raw(media: &tl::enums::MessageMedia) -> Option<TelegramWebpageInfo> {
    let wp_media = match media {
        tl::enums::MessageMedia::WebPage(w) => w,
        _ => return None,
    };
    let wp = match &wp_media.webpage {
        tl::enums::WebPage::Page(w) => w,
        _ => return None,
    };
    Some(TelegramWebpageInfo {
        url: wp.url.clone(),
        site_name: wp.site_name.clone(),
        title: wp.title.clone(),
        description: wp.description.clone(),
        embed_url: wp.embed_url.clone(),
        embed_type: wp.r#type.clone(),
        duration_sec: wp.duration,
    })
}

fn media_filter(media_type: Option<&str>) -> tl::enums::MessagesFilter {
    match media_type {
        Some("photo") => tl::enums::MessagesFilter::InputMessagesFilterPhotos,
        Some("video") => tl::enums::MessagesFilter::InputMessagesFilterVideo,
        Some("document") => tl::enums::MessagesFilter::InputMessagesFilterDocument,
        Some("audio") => tl::enums::MessagesFilter::InputMessagesFilterMusic,
        Some("webpage") => tl::enums::MessagesFilter::InputMessagesFilterUrl,
        Some("gif") => tl::enums::MessagesFilter::InputMessagesFilterGif,
        Some("round_video") => tl::enums::MessagesFilter::InputMessagesFilterRoundVideo,
        Some("round_voice") => tl::enums::MessagesFilter::InputMessagesFilterRoundVoice,
        Some("photo_video") => tl::enums::MessagesFilter::InputMessagesFilterPhotoVideo,
        Some("chat_photos") => tl::enums::MessagesFilter::InputMessagesFilterChatPhotos,
        _ => tl::enums::MessagesFilter::InputMessagesFilterEmpty,
    }
}

pub fn extract_raw_media_info(
    media: &tl::enums::MessageMedia,
    fix_extensions: bool,
) -> Option<(String, u64, String)> {
    match media {
        tl::enums::MessageMedia::Photo(photo_media) => {
            let photo = match photo_media.photo.as_ref()? {
                tl::enums::Photo::Photo(p) => p,
                tl::enums::Photo::Empty(_) => return None,
            };
            let name = format!("photo_{}.jpg", photo.id);
            let size = photo
                .sizes
                .iter()
                .filter_map(|s| match s {
                    tl::enums::PhotoSize::Size(ps) => Some(ps.size as u64),
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            Some((name, size, "photo".to_string()))
        }
        tl::enums::MessageMedia::Document(doc_media) => {
            let doc = match doc_media.document.as_ref()? {
                tl::enums::Document::Document(d) => d,
                tl::enums::Document::Empty(_) => return None,
            };
            let raw_name = doc
                .attributes
                .iter()
                .find_map(|attr| {
                    if let tl::enums::DocumentAttribute::Filename(f) = attr {
                        Some(f.file_name.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| {
                    let ext = mime_to_ext(&doc.mime_type);
                    if ext.is_empty() {
                        format!("file_{}", doc.id)
                    } else {
                        format!("{}{}", doc.id, ext)
                    }
                });
            let name = if fix_extensions {
                ensure_extension(&raw_name, &doc.mime_type)
            } else {
                raw_name
            };
            let size = doc.size as u64;

            let mut is_voice = false;
            let mut is_round_video = false;
            let mut is_animated = false;
            for attr in &doc.attributes {
                match attr {
                    tl::enums::DocumentAttribute::Audio(a) => {
                        if a.voice {
                            is_voice = true;
                        }
                    }
                    tl::enums::DocumentAttribute::Video(v) => {
                        if v.round_message {
                            is_round_video = true;
                        }
                    }
                    tl::enums::DocumentAttribute::Animated => {
                        is_animated = true;
                    }
                    _ => {}
                }
            }

            let mt = if is_voice {
                "voice"
            } else if is_round_video {
                "round_video"
            } else if is_animated {
                "gif"
            } else if doc.mime_type.starts_with("video/") {
                "video"
            } else if doc.mime_type.starts_with("audio/") {
                "audio"
            } else if doc.mime_type.starts_with("image/") {
                "photo"
            } else {
                "document"
            };
            Some((name, size, mt.to_string()))
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatPageOffset {
    pub date: i32,
    pub id: i32,
    pub peer_id: i64,
    pub peer_type: String,
    pub peer_hash: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatPage {
    pub chats: Vec<TelegramChat>,
    pub next_offset: Option<ChatPageOffset>,
}

fn media_preview_label(media: &Option<tl::enums::MessageMedia>) -> &'static str {
    match media {
        Some(tl::enums::MessageMedia::Photo(_)) => "📷 Photo",
        Some(tl::enums::MessageMedia::Document(_)) => "📎 File",
        Some(tl::enums::MessageMedia::WebPage(_)) => "🔗 Link",
        Some(tl::enums::MessageMedia::Geo(_)) => "📍 Location",
        Some(tl::enums::MessageMedia::Contact(_)) => "👤 Contact",
        Some(tl::enums::MessageMedia::Poll(_)) => "📊 Poll",
        Some(tl::enums::MessageMedia::Game(_)) => "🎮 Game",
        Some(tl::enums::MessageMedia::Venue(_)) => "📍 Venue",
        Some(_) => "📦 Media",
        None => "",
    }
}

pub async fn list_chats_page(
    handle: &TelegramSessionHandle,
    offset_date: i32,
    offset_id: i32,
    offset_peer: Option<(i64, String, i64)>,
    limit: i32,
) -> anyhow::Result<ChatPage> {
    let _t = std::time::Instant::now();
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    drop(guard);

    let limit = limit.clamp(1, 100);
    let input_peer = match offset_peer {
        Some((id, ref t, hash)) => make_input_peer(id, t, hash),
        None => tl::enums::InputPeer::Empty,
    };

    let req = tl::functions::messages::GetDialogs {
        exclude_pinned: false,
        folder_id: None,
        offset_date,
        offset_id,
        offset_peer: input_peer,
        limit,
        hash: 0,
    };

    let response = invoke_with_flood_wait(&client, &req)
        .await
        .map_err(|e| anyhow::anyhow!("messages.getDialogs: {}", e))?;

    let (dialogs, messages, chats_raw, users_raw) = match response {
        tl::enums::messages::Dialogs::Dialogs(d) => (d.dialogs, d.messages, d.chats, d.users),
        tl::enums::messages::Dialogs::Slice(d) => (d.dialogs, d.messages, d.chats, d.users),
        tl::enums::messages::Dialogs::NotModified(_) => {
            return Ok(ChatPage {
                chats: Vec::new(),
                next_offset: None,
            });
        }
    };

    let mut user_map: std::collections::HashMap<i64, (String, i64, bool, bool)> =
        std::collections::HashMap::new();
    let mut photo_id_map: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    for user in &users_raw {
        if let tl::enums::User::User(u) = user {
            let title = match (u.first_name.as_deref(), u.last_name.as_deref()) {
                (Some(f), Some(l)) if !l.is_empty() => format!("{} {}", f, l),
                (Some(f), _) => f.to_string(),
                _ => format!("user_{}", u.id),
            };
            let online = matches!(&u.status, Some(tl::enums::UserStatus::Online(_)));
            user_map.insert(
                u.id,
                (title, u.access_hash.unwrap_or(0), u.verified, online),
            );
            if let Some(tl::enums::UserProfilePhoto::Photo(p)) = &u.photo {
                photo_id_map.insert(u.id, p.photo_id);
            }
        }
    }

    let mut chat_map: std::collections::HashMap<i64, (String, String, i64, bool, bool)> =
        std::collections::HashMap::new();
    for chat in &chats_raw {
        match chat {
            tl::enums::Chat::Chat(c) => {
                chat_map.insert(
                    c.id,
                    ("group".to_string(), c.title.clone(), 0, false, false),
                );
                if let tl::enums::ChatPhoto::Photo(p) = &c.photo {
                    photo_id_map.insert(c.id, p.photo_id);
                }
            }
            tl::enums::Chat::Channel(c) => {
                let kind = if c.broadcast { "channel" } else { "group" };
                chat_map.insert(
                    c.id,
                    (
                        kind.to_string(),
                        c.title.clone(),
                        c.access_hash.unwrap_or(0),
                        c.verified,
                        false,
                    ),
                );
                if let tl::enums::ChatPhoto::Photo(p) = &c.photo {
                    photo_id_map.insert(c.id, p.photo_id);
                }
            }
            _ => {}
        }
    }

    let mut last_msg_map: std::collections::HashMap<(String, i64), (i32, i32, String)> =
        std::collections::HashMap::new();
    for m in &messages {
        let (peer_kind, peer_id, msg_id, msg_date, preview) = match m {
            tl::enums::Message::Message(msg) => {
                let (k, id) = peer_kind_id(&msg.peer_id);
                let preview = if !msg.message.trim().is_empty() {
                    msg.message.clone()
                } else {
                    media_preview_label(&msg.media).to_string()
                };
                (k, id, msg.id, msg.date, preview)
            }
            tl::enums::Message::Service(msg) => {
                let (k, id) = peer_kind_id(&msg.peer_id);
                (k, id, msg.id, msg.date, String::new())
            }
            _ => continue,
        };
        last_msg_map.insert((peer_kind, peer_id), (msg_id, msg_date, preview));
    }

    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i32)
        .unwrap_or(0);

    let mut chats = Vec::with_capacity(dialogs.len());
    let mut peer_hashes_local: std::collections::HashMap<i64, i64> =
        std::collections::HashMap::new();
    let mut last_dialog_peer: Option<(i64, String, i64)> = None;
    let mut last_dialog_msg_id: i32 = 0;
    let mut last_dialog_msg_date: i32 = 0;

    for dialog_e in &dialogs {
        let dlg = match dialog_e {
            tl::enums::Dialog::Dialog(d) => d,
            _ => continue,
        };
        let (kind, peer_id) = peer_kind_id(&dlg.peer);
        let (chat_type, title, access_hash, verified, online) = match kind.as_str() {
            "private" => match user_map.get(&peer_id) {
                Some((t, h, v, o)) => ("private".to_string(), t.clone(), *h, *v, *o),
                None => continue,
            },
            "group" | "channel" => match chat_map.get(&peer_id) {
                Some((t, title, h, v, o)) => (t.clone(), title.clone(), *h, *v, *o),
                None => continue,
            },
            _ => continue,
        };
        peer_hashes_local.insert(peer_id, access_hash);
        let is_muted = match &dlg.notify_settings {
            tl::enums::PeerNotifySettings::Settings(s) => {
                s.mute_until.map(|t| t > now_ts).unwrap_or(false)
            }
        };
        let (last_message, last_message_date) = match last_msg_map.get(&(kind.clone(), peer_id)) {
            Some((_id, date, preview)) => {
                let p = if preview.is_empty() {
                    None
                } else {
                    let truncated: String = preview.chars().take(80).collect();
                    Some(truncated)
                };
                (p, Some(*date))
            }
            None => (None, None),
        };
        chats.push(TelegramChat {
            id: peer_id,
            title,
            chat_type: chat_type.clone(),
            peer_hash: access_hash,
            last_message,
            last_message_date,
            unread_count: dlg.unread_count,
            is_muted,
            is_pinned: dlg.pinned,
            is_verified: verified,
            is_online: online,
        });
        if let Some((msg_id, msg_date, _)) = last_msg_map.get(&(kind, peer_id)) {
            last_dialog_msg_id = *msg_id;
            last_dialog_msg_date = *msg_date;
            last_dialog_peer = Some((peer_id, chat_type, access_hash));
        } else {
            last_dialog_peer = Some((peer_id, chat_type, access_hash));
        }
    }

    {
        let mut guard = handle.lock().await;
        for (id, hash) in peer_hashes_local {
            guard.peer_hashes.insert(id, hash);
        }
        for (id, photo_id) in photo_id_map {
            guard.peer_photo_ids.insert(id, photo_id);
        }
    }

    let next_offset = if (chats.len() as i32) >= limit {
        last_dialog_peer.map(|(id, t, hash)| ChatPageOffset {
            date: last_dialog_msg_date,
            id: last_dialog_msg_id,
            peer_id: id,
            peer_type: t,
            peer_hash: hash,
        })
    } else {
        None
    };

    tracing::info!(
        "[tg-perf] list_chats_page: limit={} returned={} elapsed={:?} has_next={}",
        limit,
        chats.len(),
        _t.elapsed(),
        next_offset.is_some()
    );

    Ok(ChatPage { chats, next_offset })
}

pub async fn restore_peer_hashes(
    handle: &TelegramSessionHandle,
    items: Vec<(i64, i64)>,
) -> anyhow::Result<()> {
    let mut guard = handle.lock().await;
    for (id, hash) in items {
        if hash != 0 {
            guard.peer_hashes.insert(id, hash);
        }
    }
    Ok(())
}

fn peer_kind_id(peer: &tl::enums::Peer) -> (String, i64) {
    match peer {
        tl::enums::Peer::User(u) => ("private".to_string(), u.user_id),
        tl::enums::Peer::Chat(c) => ("group".to_string(), c.chat_id),
        tl::enums::Peer::Channel(c) => ("channel".to_string(), c.channel_id),
    }
}

pub async fn list_chats(handle: &TelegramSessionHandle) -> anyhow::Result<Vec<TelegramChat>> {
    let _t = std::time::Instant::now();
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    drop(guard);

    const MAX_DIALOGS: usize = 500;

    let fetch_dialogs = async {
        let mut dialogs = client.iter_dialogs();
        let mut chats = Vec::new();
        let mut peer_hashes = std::collections::HashMap::new();

        loop {
            if chats.len() >= MAX_DIALOGS {
                tracing::info!(
                    "[tg-api] list_chats: hit MAX_DIALOGS={}, stopping",
                    MAX_DIALOGS
                );
                break;
            }
            let next = match dialogs.next().await {
                Ok(opt) => opt,
                Err(e) => {
                    let err_str = e.to_string();
                    if let Some(secs) = parse_flood_wait(&err_str) {
                        if secs <= 60 && !chats.is_empty() {
                            tracing::warn!("[tg-api] list_chats FLOOD_WAIT_{}s — returning {} chats already loaded", secs, chats.len());
                            break;
                        }
                        if secs <= 60 {
                            tracing::warn!(
                                "[tg-api] list_chats FLOOD_WAIT_{}s — sleeping then retrying",
                                secs
                            );
                            tokio::time::sleep(Duration::from_secs(secs + 1)).await;
                            continue;
                        }
                        return Err(anyhow::anyhow!(
                            "Telegram asked to wait {}s before listing chats again",
                            secs
                        ));
                    }
                    return Err(anyhow::anyhow!("{}", err_str));
                }
            };
            let dialog = match next {
                Some(d) => d,
                None => break,
            };
            let peer = dialog.peer();
            let chat_type = match peer {
                Peer::User(_) => "private",
                Peer::Group(_) => "group",
                Peer::Channel(_) => "channel",
            };
            let title = peer.name().unwrap_or("Unknown").to_string();

            let peer_data = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                use grammers_client::session::defs::PeerRef;
                let peer_ref = PeerRef::from(peer);
                (peer_ref.id.bare_id(), peer_ref.auth.hash())
            }));

            let (id, access_hash) = match peer_data {
                Ok(data) => data,
                Err(_) => {
                    tracing::warn!(
                        "[tg-api] skipping peer with unsupported ID (title={:?})",
                        title
                    );
                    continue;
                }
            };

            peer_hashes.insert(id, access_hash);

            chats.push(TelegramChat {
                id,
                title,
                chat_type: chat_type.to_string(),
                peer_hash: access_hash,
                last_message: None,
                last_message_date: None,
                unread_count: 0,
                is_muted: false,
                is_pinned: false,
                is_verified: false,
                is_online: false,
            });
        }

        Ok::<_, anyhow::Error>((chats, peer_hashes))
    };

    let (chats, peer_hashes) = tokio::time::timeout(Duration::from_secs(90), fetch_dialogs)
        .await
        .map_err(|_| anyhow::anyhow!("Loading chats timed out — please try again"))??;

    tracing::info!(
        "[tg-perf] list_chats completed in {:?}, loaded {} chats",
        _t.elapsed(),
        chats.len()
    );

    let mut guard = handle.lock().await;
    guard.peer_hashes = peer_hashes;
    drop(guard);

    Ok(chats)
}

/// Try to resolve a channel's access_hash when missing from local cache.
/// Strategy: search the dialog list briefly. If the channel appears, we'll have its hash.
/// Returns None when the channel is not in the user's dialogs.
async fn scan_dialogs_folder_for_channel(
    client: &Client,
    chat_id: i64,
    folder_id: Option<i32>,
) -> anyhow::Result<Option<i64>> {
    const PAGE_LIMIT: i32 = 100;
    const MAX_PAGES: usize = 20;

    let mut offset_date: i32 = 0;
    let mut offset_id: i32 = 0;
    let mut offset_peer: tl::enums::InputPeer = tl::enums::InputPeer::Empty;

    for page in 0..MAX_PAGES {
        let req = tl::functions::messages::GetDialogs {
            exclude_pinned: false,
            folder_id,
            offset_date,
            offset_id,
            offset_peer: offset_peer.clone(),
            limit: PAGE_LIMIT,
            hash: 0,
        };
        let resp = invoke_with_flood_wait(client, &req).await.map_err(|e| {
            anyhow::anyhow!("getDialogs(folder={:?}, page={}): {}", folder_id, page, e)
        })?;

        let (chats, dialogs, messages, complete) = match resp {
            tl::enums::messages::Dialogs::Dialogs(d) => (d.chats, d.dialogs, d.messages, true),
            tl::enums::messages::Dialogs::Slice(d) => (d.chats, d.dialogs, d.messages, false),
            tl::enums::messages::Dialogs::NotModified(_) => return Ok(None),
        };

        for chat in &chats {
            if let tl::enums::Chat::Channel(c) = chat {
                if c.id == chat_id {
                    if let Some(hash) = c.access_hash {
                        tracing::info!(
                            "[tg-api] resolve: found channel {} in folder={:?} page={} access_hash={}",
                            chat_id, folder_id, page, hash
                        );
                        return Ok(Some(hash));
                    }
                }
            }
        }

        if complete || dialogs.is_empty() {
            break;
        }

        let last_dialog = match dialogs.last() {
            Some(tl::enums::Dialog::Dialog(d)) => d,
            _ => break,
        };

        let last_msg_id = last_dialog.top_message;
        let last_msg = messages.iter().find_map(|m| {
            let id = match m {
                tl::enums::Message::Empty(e) => e.id,
                tl::enums::Message::Message(m) => m.id,
                tl::enums::Message::Service(s) => s.id,
            };
            if id == last_msg_id {
                Some(m.clone())
            } else {
                None
            }
        });

        offset_id = last_msg_id;
        offset_date = match &last_msg {
            Some(tl::enums::Message::Message(m)) => m.date,
            Some(tl::enums::Message::Service(s)) => s.date,
            _ => 0,
        };

        offset_peer = match &last_dialog.peer {
            tl::enums::Peer::User(p) => {
                let hash = chats
                    .iter()
                    .find_map(|c| match c {
                        tl::enums::Chat::Channel(_) => None,
                        _ => None,
                    })
                    .or(Some(0))
                    .unwrap_or(0);
                let _ = hash;
                tl::enums::InputPeer::User(tl::types::InputPeerUser {
                    user_id: p.user_id,
                    access_hash: 0,
                })
            }
            tl::enums::Peer::Chat(p) => {
                tl::enums::InputPeer::Chat(tl::types::InputPeerChat { chat_id: p.chat_id })
            }
            tl::enums::Peer::Channel(p) => {
                let hash = chats
                    .iter()
                    .find_map(|c| match c {
                        tl::enums::Chat::Channel(c) if c.id == p.channel_id => c.access_hash,
                        _ => None,
                    })
                    .unwrap_or(0);
                tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                    channel_id: p.channel_id,
                    access_hash: hash,
                })
            }
        };
    }

    Ok(None)
}

pub fn is_channel_invalid_error(msg: &str) -> bool {
    let upper = msg.to_uppercase();
    upper.contains("CHANNEL_INVALID")
        || upper.contains("CHANNEL_PRIVATE")
        || upper.contains("PEER_ID_INVALID")
        || upper.contains("ACCESS_HASH_INVALID")
        || upper.contains("CHAT_FORBIDDEN")
}

pub async fn refresh_channel_hash(
    handle: &TelegramSessionHandle,
    chat_id: i64,
) -> anyhow::Result<Option<i64>> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    drop(guard);

    let hash = resolve_channel_access_hash(&client, chat_id).await?;
    if let Some(h) = hash {
        let mut g = handle.lock().await;
        g.peer_hashes.insert(chat_id, h);
        tracing::info!(
            "[tg-api] refresh_channel_hash: chat={} hash refreshed",
            chat_id
        );
    }
    Ok(hash)
}

pub async fn refresh_all_dialogs(handle: &TelegramSessionHandle) -> anyhow::Result<usize> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    drop(guard);

    let mut updated = 0usize;
    for archived in [false, true] {
        let folder_id = if archived { Some(1) } else { Some(0) };
        let req = tl::functions::messages::GetDialogs {
            exclude_pinned: false,
            folder_id,
            offset_date: 0,
            offset_id: 0,
            offset_peer: tl::enums::InputPeer::Empty,
            limit: 100,
            hash: 0,
        };
        let resp = match client.invoke(&req).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("[tg-api] refresh_all_dialogs folder {:?}: {}", folder_id, e);
                continue;
            }
        };
        let chats = match resp {
            tl::enums::messages::Dialogs::Dialogs(d) => d.chats,
            tl::enums::messages::Dialogs::Slice(d) => d.chats,
            tl::enums::messages::Dialogs::NotModified(_) => Vec::new(),
        };
        let mut g = handle.lock().await;
        for c in &chats {
            if let tl::enums::Chat::Channel(ch) = c {
                if let Some(hash) = ch.access_hash {
                    let prev = g.peer_hashes.insert(ch.id, hash);
                    if prev != Some(hash) {
                        updated += 1;
                    }
                }
            }
        }
    }
    tracing::info!("[tg-api] refresh_all_dialogs: {} hashes updated", updated);
    Ok(updated)
}

pub(crate) async fn resolve_channel_access_hash(
    client: &Client,
    chat_id: i64,
) -> anyhow::Result<Option<i64>> {
    if let Some(hash) = scan_dialogs_folder_for_channel(client, chat_id, None).await? {
        return Ok(Some(hash));
    }
    if let Some(hash) = scan_dialogs_folder_for_channel(client, chat_id, Some(1)).await? {
        return Ok(Some(hash));
    }
    tracing::warn!(
        "[tg-api] resolve: channel {} not found in main or archived folders",
        chat_id
    );
    Ok(None)
}

pub fn make_input_peer(chat_id: i64, chat_type: &str, access_hash: i64) -> tl::enums::InputPeer {
    match chat_type {
        "private" => tl::enums::InputPeer::User(tl::types::InputPeerUser {
            user_id: chat_id,
            access_hash,
        }),
        "group" => {
            if access_hash != 0 {
                tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                    channel_id: chat_id,
                    access_hash,
                })
            } else {
                tl::enums::InputPeer::Chat(tl::types::InputPeerChat { chat_id })
            }
        }
        "channel" => tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
            channel_id: chat_id,
            access_hash,
        }),
        _ => tl::enums::InputPeer::Empty,
    }
}

async fn fetch_media_page(
    client: &Client,
    input_peer: &tl::enums::InputPeer,
    media_type: Option<&str>,
    query: &str,
    offset: i32,
    limit: i32,
    fix_extensions: bool,
) -> anyhow::Result<Vec<TelegramMediaItem>> {
    let filter = media_filter(media_type);
    let request = tl::functions::messages::Search {
        peer: input_peer.clone(),
        q: query.to_string(),
        from_id: None,
        saved_peer_id: None,
        saved_reaction: None,
        top_msg_id: None,
        filter,
        min_date: 0,
        max_date: 0,
        offset_id: offset,
        add_offset: 0,
        limit,
        max_id: 0,
        min_id: 0,
        hash: 0,
    };

    let result = match invoke_with_flood_wait(client, &request).await {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("CHAT_ADMIN_REQUIRED") {
                tracing::warn!(
                    "[tg-api] messages.Search filter requires admin ({}); returning empty page",
                    msg
                );
                return Ok(Vec::new());
            }
            return Err(anyhow::anyhow!("messages.Search failed: {}", msg));
        }
    };

    let messages = match result {
        tl::enums::messages::Messages::Messages(m) => m.messages,
        tl::enums::messages::Messages::Slice(m) => m.messages,
        tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
        tl::enums::messages::Messages::NotModified(_) => vec![],
    };

    let mut items = Vec::new();
    for raw_msg in messages {
        let msg = match raw_msg {
            tl::enums::Message::Message(m) => m,
            _ => continue,
        };
        let caption_text = if msg.message.trim().is_empty() {
            None
        } else {
            Some(msg.message.clone())
        };
        let grouped = msg.grouped_id;
        if let Some(raw_media) = msg.media {
            if let Some(wp) = extract_webpage_info_raw(&raw_media) {
                let display = wp
                    .title
                    .clone()
                    .or_else(|| wp.site_name.clone())
                    .unwrap_or_else(|| wp.url.clone());
                items.push(TelegramMediaItem {
                    message_id: msg.id,
                    file_name: display,
                    file_size: 0,
                    media_type: "webpage".to_string(),
                    date: msg.date as i64,
                    webpage: Some(wp),
                    caption: caption_text,
                    grouped_id: grouped,
                });
                continue;
            }
            if let Some((file_name, file_size, media_type_str)) =
                extract_raw_media_info(&raw_media, fix_extensions)
            {
                items.push(TelegramMediaItem {
                    message_id: msg.id,
                    file_name,
                    file_size,
                    media_type: media_type_str,
                    date: msg.date as i64,
                    webpage: None,
                    caption: caption_text,
                    grouped_id: grouped,
                });
            }
        }
    }

    Ok(items)
}

async fn fetch_media_via_history(
    client: &Client,
    input_peer: &tl::enums::InputPeer,
    offset_id: i32,
    limit: i32,
    fix_extensions: bool,
) -> anyhow::Result<Vec<TelegramMediaItem>> {
    let request = tl::functions::messages::GetHistory {
        peer: input_peer.clone(),
        offset_id,
        offset_date: 0,
        add_offset: 0,
        limit,
        max_id: 0,
        min_id: 0,
        hash: 0,
    };

    let result = match invoke_with_flood_wait(client, &request).await {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("CHAT_ADMIN_REQUIRED") {
                tracing::warn!(
                    "[tg-api] messages.GetHistory: admin required ({}); returning empty page",
                    msg
                );
                return Ok(Vec::new());
            }
            return Err(anyhow::anyhow!("messages.GetHistory failed: {}", msg));
        }
    };

    let messages = match result {
        tl::enums::messages::Messages::Messages(m) => m.messages,
        tl::enums::messages::Messages::Slice(m) => m.messages,
        tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
        tl::enums::messages::Messages::NotModified(_) => vec![],
    };

    let mut items = Vec::new();
    for raw_msg in messages {
        let msg = match raw_msg {
            tl::enums::Message::Message(m) => m,
            _ => continue,
        };
        let caption_text = if msg.message.trim().is_empty() {
            None
        } else {
            Some(msg.message.clone())
        };
        let grouped = msg.grouped_id;
        let raw_media = match msg.media {
            Some(m) => m,
            None => continue,
        };
        if let Some(wp) = extract_webpage_info_raw(&raw_media) {
            let display = wp
                .title
                .clone()
                .or_else(|| wp.site_name.clone())
                .unwrap_or_else(|| wp.url.clone());
            items.push(TelegramMediaItem {
                message_id: msg.id,
                file_name: display,
                file_size: 0,
                media_type: "webpage".to_string(),
                date: msg.date as i64,
                webpage: Some(wp),
                caption: caption_text,
                grouped_id: grouped,
            });
            continue;
        }
        if let Some((file_name, file_size, media_type_str)) =
            extract_raw_media_info(&raw_media, fix_extensions)
        {
            items.push(TelegramMediaItem {
                message_id: msg.id,
                file_name,
                file_size,
                media_type: media_type_str,
                date: msg.date as i64,
                webpage: None,
                caption: caption_text,
                grouped_id: grouped,
            });
        }
    }

    Ok(items)
}

fn merge_dedup_media(
    results: [anyhow::Result<Vec<TelegramMediaItem>>; 4],
    limit: usize,
) -> Vec<TelegramMediaItem> {
    const FILTER_LABELS: [&str; 4] = ["photo", "video", "document", "audio"];
    let mut all_items = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for (idx, r) in results.into_iter().enumerate() {
        match r {
            Ok(items) => all_items.extend(items),
            Err(e) => {
                let msg = e.to_string();
                tracing::warn!(
                    "[tg-api] list_media: filter={} failed: {}",
                    FILTER_LABELS[idx],
                    msg
                );
                errors.push(format!("{}: {}", FILTER_LABELS[idx], msg));
            }
        }
    }
    if all_items.is_empty() && !errors.is_empty() {
        tracing::error!(
            "[tg-api] list_media: ALL 4 filters failed — {} errors: {:?}",
            errors.len(),
            errors
        );
    }
    let mut seen = std::collections::HashSet::new();
    all_items.retain(|item| seen.insert(item.message_id));
    all_items.sort_by_key(|x| std::cmp::Reverse(x.date));
    all_items.truncate(limit);
    all_items
}

#[derive(Debug, Serialize)]
pub struct ListMediaDiag {
    pub chat_id: i64,
    pub chat_type: String,
    pub access_hash_cached: i64,
    pub access_hash_resolved: i64,
    pub auto_resolve_attempted: bool,
    pub auto_resolve_outcome: String,
    pub get_history_count: i32,
    pub get_history_with_media: i32,
    pub get_history_error: Option<String>,
    pub search_photo_count: i32,
    pub search_video_count: i32,
    pub search_document_count: i32,
    pub search_audio_count: i32,
    pub search_errors: Vec<String>,
    pub final_count: i32,
    pub elapsed_ms: u128,
}

pub async fn diag_list_media(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
) -> anyhow::Result<ListMediaDiag> {
    let started = std::time::Instant::now();
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let cached = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let mut access_hash = cached;
    let mut auto_resolve_attempted = false;
    let mut auto_resolve_outcome = String::from("not_attempted");
    if chat_type == "channel" && access_hash == 0 {
        auto_resolve_attempted = true;
        match resolve_channel_access_hash(&client, chat_id).await {
            Ok(Some(h)) => {
                access_hash = h;
                let mut g = handle.lock().await;
                g.peer_hashes.insert(chat_id, h);
                drop(g);
                auto_resolve_outcome = format!("resolved={}", h);
            }
            Ok(None) => auto_resolve_outcome = "not_found_in_dialogs".to_string(),
            Err(e) => auto_resolve_outcome = format!("error: {}", e),
        }
    }

    let input_peer = make_input_peer(chat_id, chat_type, access_hash);

    let history_req = tl::functions::messages::GetHistory {
        peer: input_peer.clone(),
        offset_id: 0,
        offset_date: 0,
        add_offset: 0,
        limit: 100,
        max_id: 0,
        min_id: 0,
        hash: 0,
    };
    let mut get_history_count = 0i32;
    let mut get_history_with_media = 0i32;
    let mut get_history_error: Option<String> = None;
    match invoke_with_flood_wait(&client, &history_req).await {
        Ok(resp) => {
            let messages = match resp {
                tl::enums::messages::Messages::Messages(m) => m.messages,
                tl::enums::messages::Messages::Slice(m) => m.messages,
                tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
                tl::enums::messages::Messages::NotModified(_) => vec![],
            };
            get_history_count = messages.len() as i32;
            for m in &messages {
                if let tl::enums::Message::Message(msg) = m {
                    if msg.media.is_some() {
                        get_history_with_media += 1;
                    }
                }
            }
        }
        Err(e) => get_history_error = Some(e.to_string()),
    }

    let mut search_counts = [0i32; 4];
    let mut search_errors: Vec<String> = Vec::new();
    let labels = ["photo", "video", "document", "audio"];
    for (i, label) in labels.iter().enumerate() {
        let filter = media_filter(Some(label));
        let req = tl::functions::messages::Search {
            peer: input_peer.clone(),
            q: String::new(),
            from_id: None,
            saved_peer_id: None,
            saved_reaction: None,
            top_msg_id: None,
            filter,
            min_date: 0,
            max_date: 0,
            offset_id: 0,
            add_offset: 0,
            limit: 50,
            max_id: 0,
            min_id: 0,
            hash: 0,
        };
        match invoke_with_flood_wait(&client, &req).await {
            Ok(resp) => {
                let n = match resp {
                    tl::enums::messages::Messages::Messages(m) => m.messages.len() as i32,
                    tl::enums::messages::Messages::Slice(m) => m.messages.len() as i32,
                    tl::enums::messages::Messages::ChannelMessages(m) => m.messages.len() as i32,
                    tl::enums::messages::Messages::NotModified(_) => 0,
                };
                search_counts[i] = n;
            }
            Err(e) => search_errors.push(format!("{}: {}", label, e)),
        }
    }

    let final_count = get_history_with_media.max(search_counts.iter().sum());

    Ok(ListMediaDiag {
        chat_id,
        chat_type: chat_type.to_string(),
        access_hash_cached: cached,
        access_hash_resolved: access_hash,
        auto_resolve_attempted,
        auto_resolve_outcome,
        get_history_count,
        get_history_with_media,
        get_history_error,
        search_photo_count: search_counts[0],
        search_video_count: search_counts[1],
        search_document_count: search_counts[2],
        search_audio_count: search_counts[3],
        search_errors,
        final_count,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

pub async fn list_media(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    media_type: Option<&str>,
    offset: i32,
    limit: u32,
    fix_extensions: bool,
) -> anyhow::Result<Vec<TelegramMediaItem>> {
    let _t = std::time::Instant::now();
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let mut access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    tracing::info!(
        "[tg-api] list_media: chat_id={}, type={}, hash={}, filter={:?}",
        chat_id,
        chat_type,
        access_hash,
        media_type
    );

    if chat_type == "channel" && access_hash == 0 {
        tracing::warn!(
            "[tg-api] list_media: access_hash=0 for channel {}, attempting auto-resolve via channels.GetChannels",
            chat_id
        );
        match resolve_channel_access_hash(&client, chat_id).await {
            Ok(Some(h)) => {
                access_hash = h;
                let mut g = handle.lock().await;
                g.peer_hashes.insert(chat_id, h);
                drop(g);
                tracing::info!(
                    "[tg-api] list_media: auto-resolved access_hash={} for channel {}",
                    h,
                    chat_id
                );
            }
            Ok(None) => {
                tracing::warn!(
                    "[tg-api] list_media: auto-resolve returned no hash for channel {} — search may fail",
                    chat_id
                );
            }
            Err(e) => {
                tracing::warn!(
                    "[tg-api] list_media: auto-resolve failed for channel {}: {}",
                    chat_id,
                    e
                );
            }
        }
    }

    async fn try_fetch(
        client: &grammers_client::Client,
        input_peer: &tl::enums::InputPeer,
        media_type: Option<&str>,
        offset: i32,
        limit: u32,
        fix_extensions: bool,
    ) -> anyhow::Result<Vec<TelegramMediaItem>> {
        if media_type.is_none() {
            let history_items =
                fetch_media_via_history(client, input_peer, offset, limit as i32, fix_extensions)
                    .await?;

            if !history_items.is_empty() {
                tracing::info!(
                    "[tg-api] list_media: GetHistory returned {} items",
                    history_items.len()
                );
                let mut items = history_items;
                items.sort_by_key(|x| std::cmp::Reverse(x.date));
                items.truncate(limit as usize);
                return Ok(items);
            }

            tracing::info!(
                "[tg-api] list_media: GetHistory empty — trying parallel filtered Search"
            );
            let (photos, videos, docs, audio) = tokio::join!(
                fetch_media_page(
                    client,
                    input_peer,
                    Some("photo"),
                    "",
                    offset,
                    limit as i32,
                    fix_extensions
                ),
                fetch_media_page(
                    client,
                    input_peer,
                    Some("video"),
                    "",
                    offset,
                    limit as i32,
                    fix_extensions
                ),
                fetch_media_page(
                    client,
                    input_peer,
                    Some("document"),
                    "",
                    offset,
                    limit as i32,
                    fix_extensions
                ),
                fetch_media_page(
                    client,
                    input_peer,
                    Some("audio"),
                    "",
                    offset,
                    limit as i32,
                    fix_extensions
                ),
            );
            let any_err = [&photos, &videos, &docs, &audio].iter().any(|r| r.is_err());
            let any_critical_err = [&photos, &videos, &docs, &audio]
                .iter()
                .filter_map(|r| r.as_ref().err())
                .any(|e| is_channel_invalid_error(&e.to_string()));
            if any_critical_err {
                let first_err = [&photos, &videos, &docs, &audio]
                    .iter()
                    .find_map(|r| r.as_ref().err().map(|e| e.to_string()))
                    .unwrap_or_default();
                anyhow::bail!("CHANNEL_INVALID propagated: {}", first_err);
            }
            let items = merge_dedup_media([photos, videos, docs, audio], limit as usize);
            if items.is_empty() && any_err {
                tracing::warn!(
                    "[tg-api] list_media: empty result with partial errors (likely protected channel or expired access_hash)"
                );
            }
            return Ok(items);
        }

        fetch_media_page(
            client,
            input_peer,
            media_type,
            "",
            offset,
            limit as i32,
            fix_extensions,
        )
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))
    }

    let mut input_peer = make_input_peer(chat_id, chat_type, access_hash);

    let fetch = async {
        match try_fetch(
            &client,
            &input_peer,
            media_type,
            offset,
            limit,
            fix_extensions,
        )
        .await
        {
            Ok(items) => Ok(items),
            Err(e) if chat_type == "channel" && is_channel_invalid_error(&e.to_string()) => {
                tracing::warn!(
                    "[tg-api] list_media: CHANNEL_INVALID for channel {} (hash={}), invalidating cache and re-resolving",
                    chat_id, access_hash
                );
                {
                    let mut g = handle.lock().await;
                    g.peer_hashes.remove(&chat_id);
                    drop(g);
                }
                match resolve_channel_access_hash(&client, chat_id).await {
                    Ok(Some(new_hash)) if new_hash != access_hash => {
                        let mut g = handle.lock().await;
                        g.peer_hashes.insert(chat_id, new_hash);
                        drop(g);
                        tracing::info!(
                            "[tg-api] list_media: re-resolved access_hash {} -> {} for channel {}, retrying",
                            access_hash, new_hash, chat_id
                        );
                        input_peer = make_input_peer(chat_id, chat_type, new_hash);
                        try_fetch(
                            &client,
                            &input_peer,
                            media_type,
                            offset,
                            limit,
                            fix_extensions,
                        )
                        .await
                    }
                    Ok(Some(same_hash)) => {
                        anyhow::bail!(
                            "Channel unreachable even after refreshing its access hash ({}). You are probably no longer a member, or it was deleted. Try leaving and joining again in the official Telegram app.",
                            same_hash
                        )
                    }
                    Ok(None) => {
                        anyhow::bail!(
                            "Channel unreachable: could not resolve its access hash. You may no longer be a member, or it was deleted. Try leaving and joining again in the official Telegram app."
                        )
                    }
                    Err(re) => {
                        anyhow::bail!(
                            "Falha ao re-resolver access_hash do channel: {}. Erro original: {}",
                            re,
                            e
                        )
                    }
                }
            }
            Err(e) => Err(e),
        }
    };

    let items = tokio::time::timeout(Duration::from_secs(30), fetch)
        .await
        .map_err(|_| anyhow::anyhow!("Loading media timed out — please try again"))??;

    {
        let g = handle.lock().await;
        let cache = g.metadata_cache.clone();
        drop(g);
        let now = std::time::Instant::now();
        for it in &items {
            super::auth::metadata_put(
                &cache,
                chat_id,
                it.message_id,
                super::auth::CachedMediaMeta {
                    filename: it.file_name.clone(),
                    mime: it.media_type.clone(),
                    size: it.file_size,
                    fetched_at: now,
                },
            );
        }
    }

    tracing::info!(
        "[tg-perf] list_media completed in {:?}, found {} items (cache populated)",
        _t.elapsed(),
        items.len()
    );
    Ok(items)
}

pub async fn expand_album(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
    fix_extensions: bool,
) -> anyhow::Result<Vec<TelegramMediaItem>> {
    let _t = std::time::Instant::now();
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let input_peer = make_input_peer(chat_id, chat_type, access_hash);
    let is_channel = matches!(input_peer, tl::enums::InputPeer::Channel(_));

    let raw_msg = if is_channel {
        let req = tl::functions::channels::GetMessages {
            channel: tl::enums::InputChannel::Channel(tl::types::InputChannel {
                channel_id: chat_id,
                access_hash,
            }),
            id: vec![tl::enums::InputMessage::Id(tl::types::InputMessageId {
                id: message_id,
            })],
        };
        let resp = invoke_with_flood_wait(&client, &req)
            .await
            .map_err(|e| anyhow::anyhow!("channels.getMessages: {}", e))?;
        match resp {
            tl::enums::messages::Messages::ChannelMessages(m) => m.messages.into_iter().next(),
            tl::enums::messages::Messages::Messages(m) => m.messages.into_iter().next(),
            tl::enums::messages::Messages::Slice(m) => m.messages.into_iter().next(),
            _ => None,
        }
    } else {
        let req = tl::functions::messages::GetMessages {
            id: vec![tl::enums::InputMessage::Id(tl::types::InputMessageId {
                id: message_id,
            })],
        };
        let resp = invoke_with_flood_wait(&client, &req)
            .await
            .map_err(|e| anyhow::anyhow!("messages.getMessages: {}", e))?;
        match resp {
            tl::enums::messages::Messages::Messages(m) => m.messages.into_iter().next(),
            tl::enums::messages::Messages::Slice(m) => m.messages.into_iter().next(),
            _ => None,
        }
    };

    let target_grouped = match raw_msg.as_ref() {
        Some(tl::enums::Message::Message(m)) => m.grouped_id,
        _ => None,
    };

    if target_grouped.is_none() {
        let single = if let Some(tl::enums::Message::Message(m)) = raw_msg {
            let caption_text = if m.message.trim().is_empty() {
                None
            } else {
                Some(m.message.clone())
            };
            let grouped = m.grouped_id;
            if let Some(raw_media) = m.media {
                if let Some(wp) = extract_webpage_info_raw(&raw_media) {
                    let display = wp
                        .title
                        .clone()
                        .or_else(|| wp.site_name.clone())
                        .unwrap_or_else(|| wp.url.clone());
                    vec![TelegramMediaItem {
                        message_id: m.id,
                        file_name: display,
                        file_size: 0,
                        media_type: "webpage".to_string(),
                        date: m.date as i64,
                        webpage: Some(wp),
                        caption: caption_text,
                        grouped_id: grouped,
                    }]
                } else if let Some((file_name, file_size, media_type_str)) =
                    extract_raw_media_info(&raw_media, fix_extensions)
                {
                    vec![TelegramMediaItem {
                        message_id: m.id,
                        file_name,
                        file_size,
                        media_type: media_type_str,
                        date: m.date as i64,
                        webpage: None,
                        caption: caption_text,
                        grouped_id: grouped,
                    }]
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        return Ok(single);
    }

    let target_grouped = target_grouped.unwrap();

    let history_req = tl::functions::messages::GetHistory {
        peer: input_peer.clone(),
        offset_id: message_id + 11,
        offset_date: 0,
        add_offset: 0,
        limit: 22,
        max_id: 0,
        min_id: 0,
        hash: 0,
    };
    let history = invoke_with_flood_wait(&client, &history_req)
        .await
        .map_err(|e| anyhow::anyhow!("messages.getHistory: {}", e))?;
    let messages = match history {
        tl::enums::messages::Messages::Messages(m) => m.messages,
        tl::enums::messages::Messages::Slice(m) => m.messages,
        tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
        tl::enums::messages::Messages::NotModified(_) => vec![],
    };

    let mut items: Vec<TelegramMediaItem> = Vec::new();
    for raw in messages {
        let msg = match raw {
            tl::enums::Message::Message(m) => m,
            _ => continue,
        };
        if msg.grouped_id != Some(target_grouped) {
            continue;
        }
        let caption_text = if msg.message.trim().is_empty() {
            None
        } else {
            Some(msg.message.clone())
        };
        let grouped = msg.grouped_id;
        if let Some(raw_media) = msg.media {
            if let Some(wp) = extract_webpage_info_raw(&raw_media) {
                let display = wp
                    .title
                    .clone()
                    .or_else(|| wp.site_name.clone())
                    .unwrap_or_else(|| wp.url.clone());
                items.push(TelegramMediaItem {
                    message_id: msg.id,
                    file_name: display,
                    file_size: 0,
                    media_type: "webpage".to_string(),
                    date: msg.date as i64,
                    webpage: Some(wp),
                    caption: caption_text,
                    grouped_id: grouped,
                });
            } else if let Some((file_name, file_size, media_type_str)) =
                extract_raw_media_info(&raw_media, fix_extensions)
            {
                items.push(TelegramMediaItem {
                    message_id: msg.id,
                    file_name,
                    file_size,
                    media_type: media_type_str,
                    date: msg.date as i64,
                    webpage: None,
                    caption: caption_text,
                    grouped_id: grouped,
                });
            }
        }
    }

    items.sort_by_key(|i| i.message_id);
    tracing::info!(
        "[tg-perf] expand_album completed in {:?}, grouped_id={}, found {} siblings",
        _t.elapsed(),
        target_grouped,
        items.len()
    );
    Ok(items)
}

pub async fn search_media(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    query: &str,
    media_type: Option<&str>,
    limit: u32,
    fix_extensions: bool,
) -> anyhow::Result<Vec<TelegramMediaItem>> {
    let _t = std::time::Instant::now();
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let input_peer = make_input_peer(chat_id, chat_type, access_hash);

    let fetch = async {
        if media_type.is_none() {
            let (photos, videos, docs, audio) = tokio::join!(
                fetch_media_page(
                    &client,
                    &input_peer,
                    Some("photo"),
                    query,
                    0,
                    limit as i32,
                    fix_extensions
                ),
                fetch_media_page(
                    &client,
                    &input_peer,
                    Some("video"),
                    query,
                    0,
                    limit as i32,
                    fix_extensions
                ),
                fetch_media_page(
                    &client,
                    &input_peer,
                    Some("document"),
                    query,
                    0,
                    limit as i32,
                    fix_extensions
                ),
                fetch_media_page(
                    &client,
                    &input_peer,
                    Some("audio"),
                    query,
                    0,
                    limit as i32,
                    fix_extensions
                ),
            );
            let items = merge_dedup_media([photos, videos, docs, audio], limit as usize);
            return Ok(items);
        }

        fetch_media_page(
            &client,
            &input_peer,
            media_type,
            query,
            0,
            limit as i32,
            fix_extensions,
        )
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))
    };

    let items = tokio::time::timeout(Duration::from_secs(30), fetch)
        .await
        .map_err(|_| anyhow::anyhow!("Search timed out — please try again"))??;

    tracing::info!(
        "[tg-perf] search_media completed in {:?}, found {} items for query={:?}",
        _t.elapsed(),
        items.len(),
        query
    );
    Ok(items)
}

pub async fn download_media(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
    output_path: &Path,
    progress_tx: mpsc::Sender<f64>,
    cancel_token: &CancellationToken,
) -> anyhow::Result<u64> {
    download_media_ex(
        handle,
        chat_id,
        chat_type,
        message_id,
        output_path,
        progress_tx,
        cancel_token,
        false,
    )
    .await
}

/// [`download_media`] that, with `dedupe`, refuses a file already saved
/// elsewhere: the error starts with `DUPLICATE_OF:` followed by its path.
/// Every successful download is registered for later checks.
#[allow(clippy::too_many_arguments)]
pub async fn download_media_ex(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
    output_path: &Path,
    progress_tx: mpsc::Sender<f64>,
    cancel_token: &CancellationToken,
    dedupe: bool,
) -> anyhow::Result<u64> {
    let _t = std::time::Instant::now();
    let guard = handle.lock().await;
    if guard.client.is_none() {
        tracing::warn!("[tg-perf] download_media: client is None (not authenticated)");
    }
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let is_auth = client.is_authorized().await.unwrap_or(false);
    tracing::info!(
        "[tg-diag] download_media: is_authorized={}, chat_id={}, msg_id={}",
        is_auth,
        chat_id,
        message_id
    );
    if !is_auth {
        tracing::error!("[tg-diag] download_media: client not authorized");
    }

    let is_channel = chat_type == "channel" || (chat_type == "group" && access_hash != 0);
    if is_channel && access_hash == 0 {
        tracing::warn!(
            "[tg-diag] download_media: access_hash=0 for channel/supergroup chat_id={}, download will likely fail",
            chat_id
        );
    }

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let part_path = PathBuf::from(format!("{}.part", output_path.display()));
    const MAX_REF_RETRIES: u32 = 2;

    for ref_attempt in 0..=MAX_REF_RETRIES {
        // Fetched again on every attempt: a fresh message carries a fresh
        // file reference (FILE_REFERENCE_EXPIRED).
        let (raw_media, msg_date) = super::parallel_download::fetch_raw_media(
            &client,
            chat_id,
            access_hash,
            is_channel,
            message_id,
        )
        .await?;

        let media = super::parallel_download::media_to_location(&raw_media)
            .ok_or_else(|| anyhow::anyhow!("Unsupported media type"))?;
        let uid = super::index::file_uid(&raw_media);
        let saved_name = extract_raw_media_info(&raw_media, super::fix_file_extensions())
            .map(|(n, _, _)| n)
            .unwrap_or_default();

        if ref_attempt == 0 && dedupe {
            if let Some(existing) = super::dedupe::find(uid.as_deref(), media.size, &saved_name) {
                let same = existing == output_path
                    || std::fs::canonicalize(&existing).ok()
                        == std::fs::canonicalize(output_path).ok();
                if !same {
                    super::dedupe::note_avoided(media.size);
                    return Err(anyhow::anyhow!(
                        "{}{}",
                        super::dedupe::DUPLICATE_PREFIX,
                        existing.display()
                    ));
                }
            }
        }

        if ref_attempt == 0 {
            tracing::info!(
                "[tg-perf] download_media: size={}, dc={}",
                media.size,
                media.dc_id
            );
        }

        let result = super::parallel_download::download_file_resumable(
            &client,
            &media,
            &part_path,
            progress_tx.clone(),
            cancel_token,
        )
        .await;

        match result {
            Ok(_) => {
                // Only a file of the expected size is ever renamed into place;
                // a 0-byte or short file never counts as done.
                let on_disk = std::fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);
                if on_disk == 0 || (media.size > 0 && on_disk != media.size) {
                    let _ = std::fs::remove_file(&part_path);
                    let _ =
                        std::fs::remove_file(super::parallel_download::chunks_sidecar(&part_path));
                    return Err(anyhow::anyhow!(
                        "Incomplete download: {} of {} bytes, will start over",
                        on_disk,
                        media.size
                    ));
                }
                std::fs::rename(&part_path, output_path)?;
                let ts = msg_date as i64;
                if ts > 0 {
                    let file_time = filetime::FileTime::from_unix_time(ts, 0);
                    if let Err(e) = filetime::set_file_mtime(output_path, file_time) {
                        tracing::warn!("[tg-api] failed to set file time: {}", e);
                    }
                }
                super::dedupe::record(
                    uid.as_deref(),
                    on_disk,
                    &saved_name,
                    output_path,
                    chat_id,
                    message_id,
                );
                tracing::info!("[tg-perf] download_media completed in {:?}", _t.elapsed());
                return Ok(on_disk);
            }
            Err(e) => {
                if cancel_token.is_cancelled() {
                    let _ = std::fs::remove_file(&part_path);
                    let _ =
                        std::fs::remove_file(super::parallel_download::chunks_sidecar(&part_path));
                    return Err(anyhow::anyhow!("Download cancelled"));
                }
                let err_lower = e.to_string().to_lowercase();
                if err_lower.contains("file_reference") && ref_attempt < MAX_REF_RETRIES {
                    tracing::warn!(
                        "[tg-diag] download_media: FILE_REFERENCE expired, re-fetching message ({}/{})",
                        ref_attempt + 1, MAX_REF_RETRIES
                    );
                    continue;
                }
                tracing::info!("[tg-perf] download_media failed in {:?}", _t.elapsed());
                return Err(e);
            }
        }
    }

    unreachable!()
}

pub async fn download_media_with_retry(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
    output_path: &Path,
    progress_tx: mpsc::Sender<f64>,
    cancel_token: &CancellationToken,
) -> anyhow::Result<u64> {
    download_media_with_retry_ex(
        handle,
        chat_id,
        chat_type,
        message_id,
        output_path,
        progress_tx,
        cancel_token,
        false,
    )
    .await
}

/// [`download_media_with_retry`] with the duplicate check of
/// [`download_media_ex`].
#[allow(clippy::too_many_arguments)]
pub async fn download_media_with_retry_ex(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
    output_path: &Path,
    progress_tx: mpsc::Sender<f64>,
    cancel_token: &CancellationToken,
    dedupe: bool,
) -> anyhow::Result<u64> {
    let _t = std::time::Instant::now();
    const MAX_RETRIES: u32 = 5;
    const BASE_DELAY_SECS: u64 = 2;

    for attempt in 0..MAX_RETRIES {
        let tx = progress_tx.clone();
        let result = tokio::select! {
            r = download_media_ex(handle, chat_id, chat_type, message_id, output_path, tx, cancel_token, dedupe) => r,
            _ = cancel_token.cancelled() => return Err(anyhow::anyhow!("Download cancelled")),
        };

        match result {
            Ok(size) => {
                tracing::info!(
                    "[tg-perf] download_media_with_retry completed in {:?}",
                    _t.elapsed()
                );
                return Ok(size);
            }
            Err(e) => {
                let err_str = e.to_string();
                if err_str.starts_with(super::dedupe::DUPLICATE_PREFIX) {
                    return Err(e);
                }

                if let Some(secs) = parse_flood_wait(&err_str) {
                    tracing::warn!(
                        "[tg-api] FLOOD_WAIT_{} on attempt {}, waiting as asked",
                        secs,
                        attempt + 1
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(secs + 1)) => {},
                        _ = cancel_token.cancelled() => return Err(anyhow::anyhow!("Download cancelled")),
                    }
                    continue;
                } else if !is_retryable_error(&err_str) {
                    tracing::info!(
                        "[tg-perf] download_media_with_retry failed (non-retryable) in {:?}",
                        _t.elapsed()
                    );
                    return Err(e);
                }

                if attempt + 1 < MAX_RETRIES {
                    let delay = BASE_DELAY_SECS * 2u64.pow(attempt);
                    tracing::warn!(
                        "[tg-api] download attempt {} failed, retrying in {}s: {}",
                        attempt + 1,
                        delay,
                        err_str
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(delay)) => {},
                        _ = cancel_token.cancelled() => return Err(anyhow::anyhow!("Download cancelled")),
                    }
                } else {
                    tracing::info!(
                        "[tg-perf] download_media_with_retry failed (max retries) in {:?}",
                        _t.elapsed()
                    );
                    return Err(e);
                }
            }
        }
    }

    unreachable!()
}
