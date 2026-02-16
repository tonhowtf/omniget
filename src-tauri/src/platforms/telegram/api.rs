use std::path::Path;

use grammers_client::grammers_tl_types as tl;
use grammers_client::session::defs::{PeerAuth, PeerId, PeerRef};
use grammers_client::types::{Media, Peer};
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

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

fn make_peer_ref(chat_id: i64, chat_type: &str) -> anyhow::Result<PeerRef> {
    let peer_id = match chat_type {
        "private" => PeerId::user(chat_id),
        "group" => PeerId::chat(chat_id),
        "channel" => PeerId::channel(chat_id),
        _ => return Err(anyhow::anyhow!("Unknown chat type: {}", chat_type)),
    };
    Ok(PeerRef {
        id: peer_id,
        auth: PeerAuth::default(),
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

fn extract_media_info(media: &Media) -> Option<(String, u64, String)> {
    match media {
        Media::Photo(photo) => {
            let name = format!("photo_{}.jpg", photo.id());
            let size = photo.size().max(0) as u64;
            Some((name, size, "photo".to_string()))
        }
        Media::Document(doc) => {
            let name = {
                let n = doc.name().to_string();
                if n.is_empty() {
                    format!("file_{}", doc.id())
                } else {
                    n
                }
            };
            let size = doc.size().max(0) as u64;
            let mt = match doc.mime_type() {
                Some(m) if m.starts_with("video/") => "video",
                Some(m) if m.starts_with("audio/") => "audio",
                _ => "document",
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
    drop(guard);

    let peer_ref = make_peer_ref(chat_id, chat_type)?;
    let filter = media_filter(media_type);

    let mut search = client.search_messages(peer_ref).filter(filter);
    if offset > 0 {
        search = search.offset_id(offset);
    }

    let mut items = Vec::new();
    while let Some(message) = search.next().await.map_err(|e| anyhow::anyhow!("{}", e))? {
        if items.len() >= limit as usize {
            break;
        }
        if let Some(media) = message.media() {
            if let Some((file_name, file_size, media_type_str)) = extract_media_info(&media) {
                items.push(TelegramMediaItem {
                    message_id: message.id(),
                    file_name,
                    file_size,
                    media_type: media_type_str,
                    date: message.date().timestamp(),
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
    drop(guard);

    let peer_ref = make_peer_ref(chat_id, chat_type)?;

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
