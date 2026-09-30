//! Walks a chat's history with filters (`tdl dl` parity): used by the whole
//! chat download, its preview and the auto-save watchers.

use std::collections::HashMap;
use std::time::Duration;

use grammers_client::grammers_tl_types as tl;
use grammers_client::Client;
use serde::{Deserialize, Serialize};

use super::api::{self, TelegramMediaItem};
use super::auth::TelegramSessionHandle;
use super::template;

/// Media types a filter understands (`webpage` previews are never files).
pub const MEDIA_TYPES: &[&str] = &[
    "video",
    "photo",
    "document",
    "audio",
    "gif",
    "round_video",
    "voice",
];

/// Which messages to take. Every field is optional; sizes in bytes, dates
/// in unix seconds, both ends inclusive.
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
    /// Accepts camelCase or snake_case keys.
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

/// Top-level `snake_case` keys become `camelCase` (`chat_id` → `chatId`),
/// so every struct can deserialize whatever the caller sent.
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
        // camelCase wins when both spellings are present.
        if out.contains_key(&key) && k.contains('_') {
            continue;
        }
        out.insert(key, val.clone());
    }
    serde_json::Value::Object(out)
}

/// The client, the peer and whether it is a channel, resolving a missing
/// channel access hash on the way (and caching it).
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

/// One message as a media item, or `None` when it carries no file.
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

/// What a scan found.
#[derive(Debug, Default)]
pub struct ScanResult {
    /// Matching media, newest first.
    pub items: Vec<TelegramMediaItem>,
    /// Newest message id seen (any kind), 0 when the chat is empty.
    pub max_id: i32,
    pub scanned: usize,
}

pub struct ScanOpts {
    /// Only messages with a larger id (auto-save's `lastMessageId`).
    pub after_id: i32,
    /// Stop after reading this many messages (0 = no cap).
    pub max_scanned: usize,
    /// Run the history requests inside a takeout session (lower limits).
    pub takeout: bool,
    pub fix_extensions: bool,
}

const PAGE: i32 = 100;

/// Reads the history newest to oldest, keeping what `filter` accepts.
/// `progress(scanned, matched)` runs after every page.
pub async fn scan_chat(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    filter: &MediaFilter,
    opts: &ScanOpts,
    mut progress: impl FnMut(usize, usize),
) -> anyhow::Result<ScanResult> {
    let (client, peer, _) = peer_for(handle, chat_id, chat_type).await?;
    let takeout_id = if opts.takeout {
        Some(api::init_takeout(handle).await?)
    } else {
        None
    };

    let from_id = filter.from_message_id.filter(|v| *v > 0).unwrap_or(0) as i32;
    let min_id = opts.after_id.max(from_id.saturating_sub(1)).max(0);
    let limit = filter.limit.filter(|l| *l > 0).map(|l| l as usize);
    let from_date = filter.from_date.map(|d| d as i64);
    let mut offset_id = filter
        .to_message_id
        .filter(|v| *v > 0)
        .map(|v| (v as i32).saturating_add(1))
        .unwrap_or(0);
    let mut offset_date = if offset_id == 0 {
        filter.to_date.map(|d| (d as i64 + 1) as i32).unwrap_or(0)
    } else {
        0
    };

    let mut out = ScanResult::default();
    'pages: loop {
        let request = tl::functions::messages::GetHistory {
            peer: peer.clone(),
            offset_id,
            offset_date,
            add_offset: 0,
            limit: PAGE,
            max_id: 0,
            min_id,
            hash: 0,
        };
        let result = match takeout_id {
            Some(tid) => api::invoke_with_takeout(&client, tid, request).await,
            None => api::invoke_with_flood_wait(&client, &request).await,
        }
        .map_err(|e| anyhow::anyhow!("messages.GetHistory: {}", e))?;
        let messages = messages_of(result);
        if messages.is_empty() {
            break;
        }
        let mut lowest = i32::MAX;
        let mut reached_start = false;
        for raw in &messages {
            let (id, date) = match raw {
                tl::enums::Message::Message(m) => (m.id, m.date as i64),
                tl::enums::Message::Service(m) => (m.id, m.date as i64),
                tl::enums::Message::Empty(m) => (m.id, 0),
            };
            lowest = lowest.min(id);
            out.max_id = out.max_id.max(id);
            if id <= min_id {
                reached_start = true;
                continue;
            }
            if let Some(from) = from_date {
                if date > 0 && date < from {
                    reached_start = true;
                    continue;
                }
            }
            out.scanned += 1;
            let tl::enums::Message::Message(m) = raw else {
                continue;
            };
            if let Some(item) = to_item(m, opts.fix_extensions) {
                if filter.matches(&item) {
                    out.items.push(item);
                    if limit.is_some_and(|l| out.items.len() >= l) {
                        progress(out.scanned, out.items.len());
                        break 'pages;
                    }
                }
            }
        }
        progress(out.scanned, out.items.len());
        if reached_start
            || lowest <= 1
            || lowest == i32::MAX
            || (opts.max_scanned > 0 && out.scanned >= opts.max_scanned)
        {
            break;
        }
        offset_id = lowest;
        offset_date = 0;
        if takeout_id.is_none() {
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    if takeout_id.is_some() {
        let _ = api::finish_takeout(handle, true).await;
    }
    Ok(out)
}

/// Id of the newest message in the chat (0 when empty).
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

/// Media data (date, caption, album, …) for a set of message ids, in
/// requests of 100. Ids without media are left out.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i32, name: &str, size: u64, kind: &str) -> TelegramMediaItem {
        TelegramMediaItem {
            message_id: id,
            file_name: name.into(),
            file_size: size,
            media_type: kind.into(),
            date: 1_700_000_000,
            webpage: None,
            caption: Some("Lecture Two".into()),
            grouped_id: None,
        }
    }

    #[test]
    fn filter_accepts_snake_and_camel() {
        let f = MediaFilter::from_json(Some(&serde_json::json!({
            "media_types": ["video"], "minSize": 10, "ext_exclude": [".MKV"], "keyword": "lecture"
        })))
        .unwrap();
        assert!(f.matches(&item(1, "a.mp4", 20, "video")));
        assert!(!f.matches(&item(2, "a.mkv", 20, "video")));
        assert!(!f.matches(&item(3, "a.mp4", 5, "video")));
        assert!(!f.matches(&item(4, "a.jpg", 20, "photo")));
    }
}
