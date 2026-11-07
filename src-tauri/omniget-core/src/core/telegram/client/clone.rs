use std::path::{Path, PathBuf};
use std::sync::Arc;

use grammers_client::grammers_tl_types as tl;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::accounts::now_unix;
use super::auth::TelegramSessionHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneOptions {
    pub delay_ms: u32,
    pub batch_size: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    pub drop_author: bool,
    pub drop_captions: bool,
}

impl Default for CloneOptions {
    fn default() -> Self {
        Self {
            delay_ms: 2000,
            batch_size: 50,
            limit: None,
            drop_author: false,
            drop_captions: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneSession {
    pub id: String,
    pub source_chat_id: i64,
    pub source_chat_type: String,
    pub source_title: String,
    pub dest_chat_id: i64,
    pub dest_chat_type: String,
    pub dest_title: String,
    pub last_message_id: i32,
    pub total_collected: i32,
    pub cloned_count: i32,
    pub failed_count: i32,
    pub status: String,
    pub options: CloneOptions,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn sessions_dir(plugin_data_dir: &Path) -> PathBuf {
    plugin_data_dir.join("clone_sessions")
}

fn session_path(plugin_data_dir: &Path, id: &str) -> PathBuf {
    sessions_dir(plugin_data_dir).join(format!("{}.json", id))
}

pub fn save_session(plugin_data_dir: &Path, s: &CloneSession) -> anyhow::Result<()> {
    let dir = sessions_dir(plugin_data_dir);
    std::fs::create_dir_all(&dir)?;
    let path = session_path(plugin_data_dir, &s.id);
    std::fs::write(&path, serde_json::to_string_pretty(s)?)?;
    Ok(())
}

pub fn load_session(plugin_data_dir: &Path, id: &str) -> anyhow::Result<CloneSession> {
    let path = session_path(plugin_data_dir, id);
    let raw = std::fs::read_to_string(&path)?;
    let s: CloneSession = serde_json::from_str(&raw)?;
    Ok(s)
}

pub fn list_sessions(plugin_data_dir: &Path) -> Vec<CloneSession> {
    let dir = sessions_dir(plugin_data_dir);
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(it) => it,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(s) = serde_json::from_str::<CloneSession>(&raw) {
                out.push(s);
            }
        }
    }
    out.sort_by_key(|x| std::cmp::Reverse(x.updated_at));
    out
}

pub fn delete_session(plugin_data_dir: &Path, id: &str) -> anyhow::Result<()> {
    let path = session_path(plugin_data_dir, id);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

type SessionMap = std::collections::HashMap<String, CancellationToken>;

fn registry() -> &'static Arc<Mutex<SessionMap>> {
    use std::sync::OnceLock;
    static R: OnceLock<Arc<Mutex<SessionMap>>> = OnceLock::new();
    R.get_or_init(|| Arc::new(Mutex::new(SessionMap::new())))
}

pub async fn cancel_session(id: &str) -> bool {
    let map = registry().lock().await;
    if let Some(token) = map.get(id) {
        token.cancel();
        true
    } else {
        false
    }
}

#[derive(Serialize, Clone)]
struct CloneProgressEvent<'a> {
    session_id: &'a str,
    stage: &'a str,
    total: i32,
    current: i32,
    failed: i32,
    last_message_id: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

fn emit_progress(session_id: &str, stage: &str, s: &CloneSession, err: Option<&str>) {
    let ev = CloneProgressEvent {
        session_id,
        stage,
        total: s.total_collected,
        current: s.cloned_count,
        failed: s.failed_count,
        last_message_id: s.last_message_id,
        error: err,
    };
    super::events::emit(
        "telegram-clone-progress",
        serde_json::to_value(&ev).unwrap_or_default(),
    );
}

pub async fn fetch_all_message_ids(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    start_after_id: i32,
    cancel: &CancellationToken,
) -> anyhow::Result<Vec<i32>> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let peer = super::api::make_input_peer(chat_id, chat_type, access_hash);
    let mut ids = Vec::<i32>::new();
    let mut offset_id = 0i32;

    loop {
        if cancel.is_cancelled() {
            return Err(anyhow::anyhow!("cancelled"));
        }
        let req = tl::functions::messages::GetHistory {
            peer: peer.clone(),
            offset_id,
            offset_date: 0,
            add_offset: 0,
            limit: 100,
            max_id: 0,
            min_id: 0,
            hash: 0,
        };
        let resp = client
            .invoke(&req)
            .await
            .map_err(|e| anyhow::anyhow!("messages.getHistory: {}", e))?;
        let messages = match resp {
            tl::enums::messages::Messages::Messages(m) => m.messages,
            tl::enums::messages::Messages::Slice(m) => m.messages,
            tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
            tl::enums::messages::Messages::NotModified(_) => vec![],
        };
        if messages.is_empty() {
            break;
        }
        let mut min_seen = i32::MAX;
        let mut found_any = false;
        for msg in &messages {
            let mid = match msg {
                tl::enums::Message::Message(m) => m.id,
                tl::enums::Message::Service(_) => continue,
                _ => continue,
            };
            if mid < min_seen {
                min_seen = mid;
            }
            if mid > start_after_id {
                ids.push(mid);
                found_any = true;
            }
        }
        if min_seen == i32::MAX {
            break;
        }
        if min_seen <= start_after_id {
            break;
        }
        offset_id = min_seen;
        if !found_any {
            break;
        }
    }

    ids.sort();
    Ok(ids)
}

#[allow(clippy::too_many_arguments)]
pub async fn start_clone(
    handle: TelegramSessionHandle,
    plugin_data_dir: PathBuf,
    runtime_handle: tokio::runtime::Handle,
    source_chat_id: i64,
    source_chat_type: String,
    source_title: String,
    dest_chat_id: i64,
    dest_chat_type: String,
    dest_title: String,
    options: CloneOptions,
    resume_session_id: Option<String>,
) -> anyhow::Result<String> {
    let session_id = resume_session_id.unwrap_or_else(|| format!("clone-{}", now_unix()));

    let mut session = match load_session(&plugin_data_dir, &session_id) {
        Ok(s) => s,
        Err(_) => CloneSession {
            id: session_id.clone(),
            source_chat_id,
            source_chat_type: source_chat_type.clone(),
            source_title: source_title.clone(),
            dest_chat_id,
            dest_chat_type: dest_chat_type.clone(),
            dest_title: dest_title.clone(),
            last_message_id: 0,
            total_collected: 0,
            cloned_count: 0,
            failed_count: 0,
            status: "running".to_string(),
            options: options.clone(),
            created_at: now_unix(),
            updated_at: now_unix(),
            error: None,
        },
    };
    session.status = "running".to_string();
    session.error = None;
    session.updated_at = now_unix();
    save_session(&plugin_data_dir, &session)?;

    let cancel = CancellationToken::new();
    {
        let mut map = registry().lock().await;
        map.insert(session_id.clone(), cancel.clone());
    }

    let session_id_clone = session_id.clone();
    runtime_handle.spawn(async move {
        let result = run_clone(
            handle,
            plugin_data_dir.clone(),
            session_id_clone.clone(),
            cancel.clone(),
        )
        .await;

        let mut session = match load_session(&plugin_data_dir, &session_id_clone) {
            Ok(s) => s,
            Err(_) => return,
        };
        match result {
            Ok(()) => {
                session.status = "completed".to_string();
                session.updated_at = now_unix();
                let _ = save_session(&plugin_data_dir, &session);
                emit_progress(&session_id_clone, "completed", &session, None);
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("cancelled") {
                    session.status = "paused".to_string();
                    session.updated_at = now_unix();
                    let _ = save_session(&plugin_data_dir, &session);
                    emit_progress(&session_id_clone, "paused", &session, None);
                } else {
                    session.status = "error".to_string();
                    session.error = Some(msg.clone());
                    session.updated_at = now_unix();
                    let _ = save_session(&plugin_data_dir, &session);
                    emit_progress(&session_id_clone, "error", &session, Some(&msg));
                }
            }
        }
        let mut map = registry().lock().await;
        map.remove(&session_id_clone);
    });

    Ok(session_id)
}

async fn run_clone(
    handle: TelegramSessionHandle,
    plugin_data_dir: PathBuf,
    session_id: String,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let mut session = load_session(&plugin_data_dir, &session_id)?;

    emit_progress(&session_id, "fetching", &session, None);

    let pending_ids = fetch_all_message_ids(
        &handle,
        session.source_chat_id,
        &session.source_chat_type,
        session.last_message_id,
        &cancel,
    )
    .await?;

    session.total_collected = session.cloned_count + pending_ids.len() as i32;
    session.updated_at = now_unix();
    save_session(&plugin_data_dir, &session)?;

    emit_progress(&session_id, "cloning", &session, None);

    let batch_size = session.options.batch_size.clamp(1, 100) as usize;
    let limit_remaining = session.options.limit;
    let mut taken = 0u32;

    for chunk in pending_ids.chunks(batch_size) {
        if cancel.is_cancelled() {
            return Err(anyhow::anyhow!("cancelled"));
        }
        if let Some(lim) = limit_remaining {
            if taken >= lim {
                break;
            }
        }
        let chunk_capped: Vec<i32> = if let Some(lim) = limit_remaining {
            let remaining = lim.saturating_sub(taken) as usize;
            chunk.iter().take(remaining).copied().collect()
        } else {
            chunk.to_vec()
        };
        if chunk_capped.is_empty() {
            break;
        }

        let res = super::api::forward_messages(
            &handle,
            session.source_chat_id,
            &session.source_chat_type,
            session.dest_chat_id,
            &session.dest_chat_type,
            chunk_capped.clone(),
            session.options.drop_author,
            session.options.drop_captions,
        )
        .await;

        match res {
            Ok(()) => {
                let last = *chunk_capped.last().unwrap_or(&session.last_message_id);
                session.cloned_count += chunk_capped.len() as i32;
                session.last_message_id = last;
                taken += chunk_capped.len() as u32;
            }
            Err(e) => {
                let msg = e.to_string();
                let upper = msg.to_uppercase();
                if upper.contains("FLOOD_WAIT") {
                    let secs = upper
                        .split("FLOOD_WAIT_")
                        .nth(1)
                        .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(15);
                    tracing::warn!("[tg-clone] FLOOD_WAIT {}s — sleeping", secs);
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(anyhow::anyhow!("cancelled")),
                        _ = tokio::time::sleep(std::time::Duration::from_secs(secs + 1)) => {}
                    }
                    continue;
                }
                if upper.contains("CHAT_FORWARDS_RESTRICTED")
                    || upper.contains("CHAT_FORWARD_RESTRICTED")
                {
                    return Err(anyhow::anyhow!(
                        "This content is protected from forwarding. Re-upload mode is not supported yet."
                    ));
                }
                session.failed_count += chunk_capped.len() as i32;
                session.error = Some(msg);
            }
        }

        session.updated_at = now_unix();
        save_session(&plugin_data_dir, &session)?;

        emit_progress(&session_id, "cloning", &session, None);

        if session.options.delay_ms > 0 {
            tokio::select! {
                _ = cancel.cancelled() => return Err(anyhow::anyhow!("cancelled")),
                _ = tokio::time::sleep(std::time::Duration::from_millis(session.options.delay_ms as u64)) => {}
            }
        }
    }

    Ok(())
}

pub async fn create_destination_channel(
    handle: &TelegramSessionHandle,
    title: &str,
) -> anyhow::Result<(i64, i64, String)> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    drop(guard);

    let final_title = title.to_string();
    let req = tl::functions::channels::CreateChannel {
        broadcast: true,
        megagroup: false,
        for_import: false,
        forum: false,
        title: final_title.clone(),
        about: "Cloned by OmniGet".to_string(),
        geo_point: None,
        address: None,
        ttl_period: None,
    };
    let updates = client
        .invoke(&req)
        .await
        .map_err(|e| anyhow::anyhow!("channels.createChannel: {}", e))?;

    let (id, hash) = match &updates {
        tl::enums::Updates::Updates(u) => super::api::extract_channel_from_chats(&u.chats),
        tl::enums::Updates::Combined(u) => super::api::extract_channel_from_chats(&u.chats),
        _ => None,
    }
    .ok_or_else(|| anyhow::anyhow!("could not find created channel id"))?;

    {
        let mut g = handle.lock().await;
        g.peer_hashes.insert(id, hash);
    }

    Ok((id, hash, final_title))
}
