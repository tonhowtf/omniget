use std::path::{Path, PathBuf};
use std::time::Duration;

use grammers_client::Client;
use grammers_client::grammers_tl_types as tl;
use grammers_client::session::defs::PeerRef;
use grammers_client::types::Peer;
use grammers_tl_types::Serializable;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::auth::TelegramSessionHandle;

/// Maps a MIME type to a file extension (with leading dot).
/// Covers the most common Telegram media MIME types.
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
                // Common pattern: "video/mp4" → ".mp4"
                // But we only return static strings, so we use a catch-all
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

/// Ensures a filename has a proper extension based on its MIME type.
/// If the file already has a known extension, keeps it.
/// If not, appends the MIME-derived extension.
fn ensure_extension(name: &str, mime_type: &str) -> String {
    let path = Path::new(name);
    if let Some(ext) = path.extension() {
        let ext_str = ext.to_string_lossy().to_lowercase();
        // Already has a recognized extension
        let known = matches!(
            ext_str.as_str(),
            "mp4" | "mkv" | "webm" | "mov" | "avi" | "mpeg" | "3gp" | "flv"
            | "mp3" | "ogg" | "opus" | "flac" | "wav" | "aac" | "m4a" | "wma"
            | "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" | "tiff"
            | "pdf" | "zip" | "rar" | "7z" | "gz" | "tar"
            | "json" | "xml" | "txt" | "html" | "csv"
            | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx"
            | "py" | "js" | "ts" | "rs" | "go" | "c" | "cpp" | "h"
        );
        if known {
            return name.to_string();
        }
    }

    // No extension or unknown extension — add from MIME type
    let ext = mime_to_ext(mime_type);
    if ext.is_empty() {
        name.to_string()
    } else {
        format!("{}{}", name, ext)
    }
}

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

fn media_filter(media_type: Option<&str>) -> tl::enums::MessagesFilter {
    match media_type {
        Some("photo") => tl::enums::MessagesFilter::InputMessagesFilterPhotos,
        Some("video") => tl::enums::MessagesFilter::InputMessagesFilterVideo,
        Some("document") => tl::enums::MessagesFilter::InputMessagesFilterDocument,
        Some("audio") => tl::enums::MessagesFilter::InputMessagesFilterMusic,
        _ => tl::enums::MessagesFilter::InputMessagesFilterEmpty,
    }
}

fn extract_raw_media_info(media: &tl::enums::MessageMedia, fix_extensions: bool) -> Option<(String, u64, String)> {
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
            let raw_name = doc.attributes.iter().find_map(|attr| {
                if let tl::enums::DocumentAttribute::Filename(f) = attr {
                    Some(f.file_name.clone())
                } else {
                    None
                }
            }).unwrap_or_else(|| {
                // No filename attribute — use ID + MIME extension (like tdl-master)
                let ext = mime_to_ext(&doc.mime_type);
                if ext.is_empty() {
                    format!("file_{}", doc.id)
                } else {
                    format!("{}{}", doc.id, ext)
                }
            });
            // Ensure files with names but missing extensions get the right one
            let name = if fix_extensions {
                ensure_extension(&raw_name, &doc.mime_type)
            } else {
                raw_name
            };
            let size = doc.size as u64;
            let mt = if doc.mime_type.starts_with("video/") {
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

pub async fn list_chats(
    handle: &TelegramSessionHandle,
) -> anyhow::Result<Vec<TelegramChat>> {
    let _t = std::time::Instant::now();
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

    tracing::info!("[tg-perf] list_chats completed in {:?}, loaded {} chats", _t.elapsed(), chats.len());

    let mut guard = handle.lock().await;
    guard.peer_hashes = peer_hashes;
    drop(guard);

    Ok(chats)
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

    let result = invoke_with_flood_wait(client, &request).await
        .map_err(|e| anyhow::anyhow!("messages.Search failed: {}", e))?;

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
        if let Some(raw_media) = msg.media {
            if let Some((file_name, file_size, media_type_str)) = extract_raw_media_info(&raw_media, fix_extensions) {
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

fn merge_dedup_media(results: [anyhow::Result<Vec<TelegramMediaItem>>; 4], limit: usize) -> Vec<TelegramMediaItem> {
    let mut all_items = Vec::new();
    for result in results {
        if let Ok(items) = result {
            all_items.extend(items);
        }
    }
    let mut seen = std::collections::HashSet::new();
    all_items.retain(|item| seen.insert(item.message_id));
    all_items.sort_by(|a, b| b.date.cmp(&a.date));
    all_items.truncate(limit);
    all_items
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

    if media_type.is_none() {
        let (photos, videos, docs, audio) = tokio::join!(
            fetch_media_page(&client, &input_peer, Some("photo"), "", offset, limit as i32, fix_extensions),
            fetch_media_page(&client, &input_peer, Some("video"), "", offset, limit as i32, fix_extensions),
            fetch_media_page(&client, &input_peer, Some("document"), "", offset, limit as i32, fix_extensions),
            fetch_media_page(&client, &input_peer, Some("audio"), "", offset, limit as i32, fix_extensions),
        );
        let items = merge_dedup_media([photos, videos, docs, audio], limit as usize);
        tracing::info!("[tg-perf] list_media (all) completed in {:?}, found {} items", _t.elapsed(), items.len());
        return Ok(items);
    }

    let items = fetch_media_page(&client, &input_peer, media_type, "", offset, limit as i32, fix_extensions).await?;
    tracing::info!("[tg-perf] list_media completed in {:?}, found {} items", _t.elapsed(), items.len());
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
    let client = guard.client.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let input_peer = make_input_peer(chat_id, chat_type, access_hash);

    if media_type.is_none() {
        let (photos, videos, docs, audio) = tokio::join!(
            fetch_media_page(&client, &input_peer, Some("photo"), query, 0, limit as i32, fix_extensions),
            fetch_media_page(&client, &input_peer, Some("video"), query, 0, limit as i32, fix_extensions),
            fetch_media_page(&client, &input_peer, Some("document"), query, 0, limit as i32, fix_extensions),
            fetch_media_page(&client, &input_peer, Some("audio"), query, 0, limit as i32, fix_extensions),
        );
        let items = merge_dedup_media([photos, videos, docs, audio], limit as usize);
        tracing::info!("[tg-perf] search_media (all) completed in {:?}, found {} items for query={:?}", _t.elapsed(), items.len(), query);
        return Ok(items);
    }

    let items = fetch_media_page(&client, &input_peer, media_type, query, 0, limit as i32, fix_extensions).await?;
    tracing::info!("[tg-perf] search_media completed in {:?}, found {} items for query={:?}", _t.elapsed(), items.len(), query);
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
    let _t = std::time::Instant::now();
    let guard = handle.lock().await;
    if guard.client.is_none() {
        tracing::warn!("[tg-perf] download_media: client is None (not authenticated)");
    }
    let client = guard.client.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let is_auth = client.is_authorized().await.unwrap_or(false);
    tracing::info!("[tg-diag] download_media: is_authorized={}, chat_id={}, msg_id={}", is_auth, chat_id, message_id);
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
        tokio::fs::create_dir_all(parent).await?;
    }

    let tmp_path = PathBuf::from(format!("{}.tmp", output_path.display()));
    const MAX_REF_RETRIES: u32 = 2;

    for ref_attempt in 0..=MAX_REF_RETRIES {
        let (raw_media, msg_date) = super::parallel_download::fetch_raw_media(
            &client, chat_id, access_hash, is_channel, message_id,
        ).await?;

        let (location, total_size) = super::parallel_download::media_to_input_location(&raw_media)
            .ok_or_else(|| anyhow::anyhow!("Unsupported media type"))?;

        if ref_attempt == 0 {
            tracing::info!("[tg-perf] download_media: total_size={}", total_size);
        }

        let result = super::parallel_download::download_parallel(
            &client, location, total_size, &tmp_path, progress_tx.clone(), cancel_token, 8,
        ).await;

        match result {
            Ok(downloaded) => {
                tokio::fs::rename(&tmp_path, output_path).await?;
                let ts = msg_date as i64;
                if ts > 0 {
                    let file_time = filetime::FileTime::from_unix_time(ts, 0);
                    if let Err(e) = filetime::set_file_mtime(output_path, file_time) {
                        tracing::warn!("[tg-api] failed to set file time: {}", e);
                    }
                }
                tracing::info!("[tg-perf] download_media completed in {:?}", _t.elapsed());
                return Ok(downloaded);
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp_path).await;
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

fn is_retryable_error(err_str: &str) -> bool {
    let retryable = [
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
    let lower = err_str.to_lowercase();
    retryable.iter().any(|p| lower.contains(p))
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
    let _t = std::time::Instant::now();
    const MAX_RETRIES: u32 = 5;
    const BASE_DELAY_SECS: u64 = 2;

    for attempt in 0..MAX_RETRIES {
        let tx = progress_tx.clone();
        let result = tokio::select! {
            r = download_media(handle, chat_id, chat_type, message_id, output_path, tx, cancel_token) => r,
            _ = cancel_token.cancelled() => return Err(anyhow::anyhow!("Download cancelled")),
        };

        match result {
            Ok(size) => {
                tracing::info!("[tg-perf] download_media_with_retry completed in {:?}", _t.elapsed());
                return Ok(size);
            }
            Err(e) => {
                let err_str = e.to_string();

                if parse_flood_wait(&err_str).is_some() {
                    // Flood wait is handled inside invoke_with_flood_wait,
                    // but if it bubbles up, retry
                    tracing::warn!(
                        "[tg-api] flood wait error on attempt {}, retrying: {}",
                        attempt + 1, err_str
                    );
                } else if !is_retryable_error(&err_str) {
                    tracing::info!("[tg-perf] download_media_with_retry failed (non-retryable) in {:?}", _t.elapsed());
                    return Err(e);
                }

                if attempt + 1 < MAX_RETRIES {
                    let delay = BASE_DELAY_SECS * 2u64.pow(attempt);
                    tracing::warn!(
                        "[tg-api] download attempt {} failed, retrying in {}s: {}",
                        attempt + 1, delay, err_str
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(delay)) => {},
                        _ = cancel_token.cancelled() => return Err(anyhow::anyhow!("Download cancelled")),
                    }
                } else {
                    tracing::info!("[tg-perf] download_media_with_retry failed (max retries) in {:?}", _t.elapsed());
                    return Err(e);
                }
            }
        }
    }

    unreachable!()
}
