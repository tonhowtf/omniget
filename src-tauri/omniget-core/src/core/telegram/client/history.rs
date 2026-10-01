use std::collections::HashMap;

use grammers_client::grammers_tl_types as tl;
use grammers_client::Client;
use serde::{Deserialize, Serialize};

use super::api::{self, TelegramMediaItem};
use super::auth::TelegramSessionHandle;
use super::template;

pub const MEDIA_TYPES: &[&str] = &[
    "video",
    "photo",
    "document",
    "audio",
    "gif",
    "round_video",
    "voice",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MediaFilter {
    pub media_types: Option<Vec<String>>,
    pub from_date: Option<f64>,
    pub to_date: Option<f64>,
    pub min_size: Option<f64>,
    pub max_size: Option<f64>,
    pub keyword: Option<String>,
    pub ext_include: Option<Vec<String>>,
    pub ext_exclude: Option<Vec<String>>,
    pub from_message_id: Option<i64>,
    pub to_message_id: Option<i64>,
    pub limit: Option<u64>,
}

fn norm_exts(list: &Option<Vec<String>>) -> Vec<String> {
    list.as_deref()
        .unwrap_or_default()
        .iter()
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .collect()
}

impl MediaFilter {
    pub fn from_json(v: Option<&serde_json::Value>) -> Result<Self, String> {
        match v {
            None | Some(serde_json::Value::Null) => Ok(Self::default()),
            Some(v) => serde_json::from_value(camelize(v)).map_err(|e| format!("filter: {}", e)),
        }
    }

    pub fn matches(&self, it: &TelegramMediaItem) -> bool {
        if it.media_type == "webpage" {
            return false;
        }
        if let Some(types) = self.media_types.as_ref().filter(|t| !t.is_empty()) {
            if !types.iter().any(|t| t == &it.media_type) {
                return false;
            }
        }
        if let Some(from) = self.from_date {
            if (it.date as f64) < from {
                return false;
            }
        }
        if let Some(to) = self.to_date {
            if (it.date as f64) > to {
                return false;
            }
        }
        if let Some(min) = self.min_size {
            if (it.file_size as f64) < min {
                return false;
            }
        }
        if let Some(max) = self.max_size.filter(|m| *m > 0.0) {
            if (it.file_size as f64) > max {
                return false;
            }
        }
        if let Some(from) = self.from_message_id.filter(|v| *v > 0) {
            if (it.message_id as i64) < from {
                return false;
            }
        }
        if let Some(to) = self.to_message_id.filter(|v| *v > 0) {
            if (it.message_id as i64) > to {
                return false;
            }
        }
        if let Some(kw) = self
            .keyword
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        {
            let kw = kw.to_lowercase();
            let in_caption = it
                .caption
                .as_deref()
                .map(|c| c.to_lowercase().contains(&kw))
                .unwrap_or(false);
            if !in_caption && !it.file_name.to_lowercase().contains(&kw) {
                return false;
            }
        }
        let ext = template::ext_of(&it.file_name);
        let include = norm_exts(&self.ext_include);
        if !include.is_empty() && !include.contains(&ext) {
            return false;
        }
        let exclude = norm_exts(&self.ext_exclude);
        if exclude.contains(&ext) {
            return false;
        }
        true
    }
}

pub fn camelize(v: &serde_json::Value) -> serde_json::Value {
    let Some(obj) = v.as_object() else {
        return v.clone();
    };
    let mut out = serde_json::Map::new();
    for (k, val) in obj {
        let mut key = String::with_capacity(k.len());
        let mut upper = false;
        for c in k.chars() {
            if c == '_' {
                upper = true;
            } else if upper {
                key.extend(c.to_uppercase());
                upper = false;
            } else {
                key.push(c);
            }
        }
        if out.contains_key(&key) && k.contains('_') {
            continue;
        }
        out.insert(key, val.clone());
    }
    serde_json::Value::Object(out)
}

pub async fn peer_for(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
) -> anyhow::Result<(Client, tl::enums::InputPeer, i64)> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let mut access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);
    if chat_type == "channel" && access_hash == 0 {
        if let Ok(Some(h)) = api::resolve_channel_access_hash(&client, chat_id).await {
            access_hash = h;
            handle.lock().await.peer_hashes.insert(chat_id, h);
        }
    }
    let peer = api::make_input_peer(chat_id, chat_type, access_hash);
    Ok((client, peer, access_hash))
}

pub fn to_item(msg: &tl::types::Message, fix_extensions: bool) -> Option<TelegramMediaItem> {
    let media = msg.media.as_ref()?;
    if api::extract_webpage_info_raw(media).is_some() {
        return None;
    }
    let (file_name, file_size, media_type) = api::extract_raw_media_info(media, fix_extensions)?;
    Some(TelegramMediaItem {
        message_id: msg.id,
        file_name,
        file_size,
        media_type,
        date: msg.date as i64,
        webpage: None,
        caption: (!msg.message.trim().is_empty()).then(|| msg.message.clone()),
        grouped_id: msg.grouped_id,
    })
}

fn messages_of(result: tl::enums::messages::Messages) -> Vec<tl::enums::Message> {
    match result {
        tl::enums::messages::Messages::Messages(m) => m.messages,
        tl::enums::messages::Messages::Slice(m) => m.messages,
        tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
        tl::enums::messages::Messages::NotModified(_) => vec![],
    }
}

pub async fn latest_message_id(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
) -> anyhow::Result<i32> {
    let (client, peer, _) = peer_for(handle, chat_id, chat_type).await?;
    let request = tl::functions::messages::GetHistory {
        peer,
        offset_id: 0,
        offset_date: 0,
        add_offset: 0,
        limit: 1,
        max_id: 0,
        min_id: 0,
        hash: 0,
    };
    let result = api::invoke_with_flood_wait(&client, &request)
        .await
        .map_err(|e| anyhow::anyhow!("messages.GetHistory: {}", e))?;
    Ok(messages_of(result)
        .iter()
        .map(|m| match m {
            tl::enums::Message::Message(m) => m.id,
            tl::enums::Message::Service(m) => m.id,
            tl::enums::Message::Empty(m) => m.id,
        })
        .max()
        .unwrap_or(0))
}

pub async fn fetch_items(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    ids: &[i32],
    fix_extensions: bool,
) -> anyhow::Result<HashMap<i32, TelegramMediaItem>> {
    let (client, _, access_hash) = peer_for(handle, chat_id, chat_type).await?;
    let is_channel = chat_type == "channel" || (chat_type == "group" && access_hash != 0);
    let mut out = HashMap::new();
    for chunk in ids.chunks(100) {
        let input: Vec<tl::enums::InputMessage> = chunk
            .iter()
            .map(|id| tl::enums::InputMessage::Id(tl::types::InputMessageId { id: *id }))
            .collect();
        let result = if is_channel {
            let request = tl::functions::channels::GetMessages {
                channel: tl::enums::InputChannel::Channel(tl::types::InputChannel {
                    channel_id: chat_id,
                    access_hash,
                }),
                id: input,
            };
            api::invoke_with_flood_wait(&client, &request).await
        } else {
            let request = tl::functions::messages::GetMessages { id: input };
            api::invoke_with_flood_wait(&client, &request).await
        }
        .map_err(|e| anyhow::anyhow!("getMessages: {}", e))?;
        for raw in messages_of(result) {
            if let tl::enums::Message::Message(m) = raw {
                if let Some(item) = to_item(&m, fix_extensions) {
                    out.insert(item.message_id, item);
                }
            }
        }
    }
    Ok(out)
}
