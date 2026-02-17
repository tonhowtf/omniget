use std::path::Path;
use std::time::Duration;

use grammers_client::Client;
use grammers_client::grammers_tl_types as tl;
use grammers_client::session::defs::{PeerAuth, PeerId, PeerRef};
use grammers_client::types::{Media, Peer};
use grammers_tl_types::Serializable;
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use super::auth::TelegramSessionHandle;

fn parse_flood_wait(err: &str) -> Option<u64> {
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

async fn invoke_with_flood_wait<R>(client: &Client, request: &R) -> Result<R::Return, grammers_mtsender::InvocationError>
where
    R: grammers_tl_types::RemoteCall + Serializable,
{
    for attempt in 0..3u32 {
        match client.invoke(request).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                let err_str = e.to_string();
                if let Some(secs) = parse_flood_wait(&err_str) {
                    let wait = secs + 1;
                    tracing::warn!(
                        "[tg-api] FLOOD_WAIT_{} on attempt {}, waiting {}s",
                        secs, attempt + 1, wait
                    );
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                    continue;
                }
                return Err(e);
            }
        }
    }
    client.invoke(request).await
}

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

fn make_peer_ref(chat_id: i64, chat_type: &str, access_hash: i64) -> anyhow::Result<PeerRef> {
    let is_supergroup = chat_type == "group" && access_hash != 0;
    let peer_id = match chat_type {
        "private" => PeerId::user(chat_id),
        "group" if is_supergroup => PeerId::channel(chat_id),
        "group" => PeerId::chat(chat_id),
        "channel" => PeerId::channel(chat_id),
        _ => return Err(anyhow::anyhow!("Unknown chat type: {}", chat_type)),
    };
    let auth = if access_hash != 0 {
        PeerAuth::from_hash(access_hash)
    } else {
        PeerAuth::default()
    };
    Ok(PeerRef {
        id: peer_id,
        auth,
    })
}

fn media_filter(media_type: Option<&str>) -> tl::enums::MessagesFilter {
    match media_type {
        Some("photo") => tl::enums::MessagesFilter::InputMessagesFilterPhotos,
        Some("video") => tl::enums::MessagesFilter::InputMessagesFilterVideo,
        Some("document") => tl::enums::MessagesFilter::InputMessagesFilterDocument,
        Some("audio") => tl::enums::MessagesFilter::InputMessagesFilterMusic,
        _ => tl::enums::MessagesFilter::InputMessagesFilterEmpty,
    }
}

fn extract_raw_media_info(media: &tl::enums::MessageMedia) -> Option<(String, u64, String)> {
    match media {
        tl::enums::MessageMedia::Photo(photo_media) => {
            let photo = match photo_media.photo.as_ref()? {
                tl::enums::Photo::Photo(p) => p,
                tl::enums::Photo::Empty(_) => return None,
            };
            let name = format!("photo_{}.jpg", photo.id);
            let size = photo.sizes.iter().filter_map(|s| match s {
                tl::enums::PhotoSize::Size(ps) => Some(ps.size as u64),
                _ => None,
            }).max().unwrap_or(0);
            Some((name, size, "photo".to_string()))
        }
        tl::enums::MessageMedia::Document(doc_media) => {
            let doc = match doc_media.document.as_ref()? {
                tl::enums::Document::Document(d) => d,
                tl::enums::Document::Empty(_) => return None,
            };
            let name = doc.attributes.iter().find_map(|attr| {
                if let tl::enums::DocumentAttribute::Filename(f) = attr {
                    Some(f.file_name.clone())
                } else {
                    None
                }
            }).unwrap_or_else(|| format!("file_{}", doc.id));
            let size = doc.size as u64;
            let mt = if doc.mime_type.starts_with("video/") {
                "video"
            } else if doc.mime_type.starts_with("audio/") {
                "audio"
            } else {
                "document"
            };
            Some((name, size, mt.to_string()))
        }
        _ => None,
    }
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

    let mut peer_hashes = std::collections::HashMap::new();

    while let Some(dialog) = dialogs.next().await.map_err(|e| anyhow::anyhow!("{}", e))? {
        let peer = dialog.peer();
        let chat_type = match peer {
            Peer::User(_) => "private",
            Peer::Group(_) => "group",
            Peer::Channel(_) => "channel",
        };
        let peer_ref = PeerRef::from(peer);
        let id = peer_ref.id.bare_id();
        let access_hash = peer_ref.auth.hash();
        let title = peer.name().unwrap_or("Unknown").to_string();

        peer_hashes.insert(id, access_hash);

        chats.push(TelegramChat {
            id,
            title,
            chat_type: chat_type.to_string(),
        });
    }

    tracing::info!("[tg-api] list_chats: loaded {} chats", chats.len());

    let mut guard = handle.lock().await;
    guard.peer_hashes = peer_hashes;
    drop(guard);

    Ok(chats)
}

fn make_input_peer(chat_id: i64, chat_type: &str, access_hash: i64) -> tl::enums::InputPeer {
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
                tl::enums::InputPeer::Chat(tl::types::InputPeerChat {
                    chat_id,
                })
            }
        }
        "channel" => tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
            channel_id: chat_id,
            access_hash,
        }),
        _ => tl::enums::InputPeer::Empty,
    }
}

pub async fn list_media(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    media_type: Option<&str>,
    offset: i32,
    limit: u32,
) -> anyhow::Result<Vec<TelegramMediaItem>> {
    let guard = handle.lock().await;
    let client = guard.client.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    tracing::debug!(
        "[tg-api] list_media: chat_id={}, type={}, hash={}, filter={:?}",
        chat_id, chat_type, access_hash, media_type
    );

    let input_peer = make_input_peer(chat_id, chat_type, access_hash);
    let filter = media_filter(media_type);

    let request = tl::functions::messages::Search {
        peer: input_peer,
        q: String::new(),
        from_id: None,
        saved_peer_id: None,
        saved_reaction: None,
        top_msg_id: None,
        filter,
        min_date: 0,
        max_date: 0,
        offset_id: offset,
        add_offset: 0,
        limit: limit as i32,
        max_id: 0,
        min_id: 0,
        hash: 0,
    };

    let result = invoke_with_flood_wait(&client, &request).await
        .map_err(|e| {
            tracing::error!("[tg-api] messages.Search failed: {}", e);
            anyhow::anyhow!("messages.Search failed: {}", e)
        })?;

    let messages = match result {
        tl::enums::messages::Messages::Messages(m) => m.messages,
        tl::enums::messages::Messages::Slice(m) => m.messages,
        tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
        tl::enums::messages::Messages::NotModified(_) => vec![],
    };

    tracing::debug!("[tg-api] messages.Search returned {} messages", messages.len());

    let mut items = Vec::new();
    for raw_msg in messages {
        let msg = match raw_msg {
            tl::enums::Message::Message(m) => m,
            _ => continue,
        };

        if let Some(raw_media) = msg.media {
            if let Some((file_name, file_size, media_type_str)) = extract_raw_media_info(&raw_media) {
                items.push(TelegramMediaItem {
                    message_id: msg.id,
                    file_name,
                    file_size,
                    media_type: media_type_str,
                    date: msg.date as i64,
                });
            }
        }
    }

    Ok(items)
}

pub async fn download_media(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
    output_path: &Path,
    progress_tx: mpsc::Sender<f64>,
) -> anyhow::Result<u64> {
    let guard = handle.lock().await;
    let client = guard.client.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let peer_ref = make_peer_ref(chat_id, chat_type, access_hash)?;

    let messages = client
        .get_messages_by_id(peer_ref, &[message_id])
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let message = messages
        .into_iter()
        .next()
        .flatten()
        .ok_or_else(|| anyhow::anyhow!("Message not found"))?;

    let media = message
        .media()
        .ok_or_else(|| anyhow::anyhow!("Message has no media"))?;

    let total_size = match &media {
        Media::Document(doc) => doc.size().max(0) as u64,
        Media::Photo(photo) => photo.size().max(0) as u64,
        _ => 0,
    };

    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut file = tokio::fs::File::create(output_path).await?;
    let mut download = client.iter_download(&media);
    let mut downloaded: u64 = 0;

    while let Some(chunk) = download.next().await.map_err(|e| anyhow::anyhow!("{}", e))? {
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        if total_size > 0 {
            let percent = (downloaded as f64 / total_size as f64) * 100.0;
            let _ = progress_tx.send(percent.min(100.0)).await;
        }
    }

    file.flush().await?;
    Ok(downloaded)
}
