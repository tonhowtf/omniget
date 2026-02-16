use grammers_client::types::Peer;
use serde::Serialize;

use super::auth::TelegramSessionHandle;

#[derive(Debug, Clone, Serialize)]
pub struct TelegramChat {
    pub id: i64,
    pub title: String,
    pub chat_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TelegramMediaItem {
    pub message_id: i32,
    pub file_name: String,
    pub file_size: u64,
    pub media_type: String,
    pub date: i64,
}

pub async fn list_chats(
    handle: &TelegramSessionHandle,
) -> anyhow::Result<Vec<TelegramChat>> {
    let guard = handle.lock().await;
    let client = guard.client.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    drop(guard);

    let mut dialogs = client.iter_dialogs();
    let mut chats = Vec::new();

    while let Some(dialog) = dialogs.next().await.map_err(|e| anyhow::anyhow!("{}", e))? {
        let peer = dialog.peer();
        let chat_type = match peer {
            Peer::User(_) => "private",
            Peer::Group(_) => "group",
            Peer::Channel(_) => "channel",
        };
        let id = peer.id().bare_id();
        let title = peer.name().unwrap_or("Unknown").to_string();

        chats.push(TelegramChat {
            id,
            title,
            chat_type: chat_type.to_string(),
        });
    }

    Ok(chats)
}

pub async fn list_media(
    handle: &TelegramSessionHandle,
    _chat_id: i64,
    _media_type: Option<&str>,
    _offset: u32,
    _limit: u32,
) -> anyhow::Result<Vec<TelegramMediaItem>> {
    let guard = handle.lock().await;
    let _client = guard.client.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    drop(guard);

    Ok(Vec::new())
}
