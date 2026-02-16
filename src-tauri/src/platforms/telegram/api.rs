use serde::Serialize;

use super::auth::TelegramSessionHandle;

#[derive(Debug, Clone, Serialize)]
pub struct TelegramChat {
    pub id: i64,
    pub title: String,
    pub chat_type: String,
    pub media_count: Option<u32>,
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
    _filter: Option<&str>,
) -> anyhow::Result<Vec<TelegramChat>> {
    let guard = handle.lock().await;
    let _client = guard.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .client.clone();
    drop(guard);

    Ok(Vec::new())
}

pub async fn list_media(
    handle: &TelegramSessionHandle,
    _chat_id: i64,
    _media_type: Option<&str>,
    _offset: u32,
    _limit: u32,
) -> anyhow::Result<Vec<TelegramMediaItem>> {
    let guard = handle.lock().await;
    let _client = guard.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .client.clone();
    drop(guard);

    Ok(Vec::new())
}
