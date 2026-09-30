use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use grammers_client::grammers_tl_types as tl;
use grammers_client::Client;
use tokio::sync::Mutex;

use super::auth::TelegramSessionHandle;
use super::parallel_download::fetch_raw_media;
use super::preview_cache::{PreviewDiskCache, DEFAULT_MAX_BYTES, DEFAULT_MAX_FILES};

const TTL_SECS: u64 = 120;
const MAX_BYTES: u64 = 30 * 1024 * 1024;
const DISK_CACHE_THRESHOLD_BYTES: usize = 200 * 1024;

static DISK_CACHE: OnceLock<Arc<PreviewDiskCache>> = OnceLock::new();

pub fn init_disk_cache(dir: PathBuf) {
    let cache = Arc::new(PreviewDiskCache::new(
        dir,
        DEFAULT_MAX_FILES,
        DEFAULT_MAX_BYTES,
    ));
    let _ = DISK_CACHE.set(cache);
    tracing::info!("[preview-cache] initialized");
}

fn disk_cache() -> Option<Arc<PreviewDiskCache>> {
    DISK_CACHE.get().cloned()
}

fn disk_key(chat_id: i64, message_id: i32) -> String {
    format!("thumb_{}_{}", chat_id, message_id)
}

struct CacheEntry {
    data: Vec<u8>,
    inserted_at: Instant,
}

struct ThumbnailCache {
    entries: HashMap<(i64, i32), CacheEntry>,
    total_bytes: u64,
}

impl ThumbnailCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            total_bytes: 0,
        }
    }

    fn get(&mut self, key: &(i64, i32)) -> Option<Vec<u8>> {
        self.evict_expired();
        self.entries.get(key).map(|e| e.data.clone())
    }

    fn insert(&mut self, key: (i64, i32), data: Vec<u8>) {
        let data_len = data.len() as u64;

        if let Some(old) = self.entries.remove(&key) {
            self.total_bytes -= old.data.len() as u64;
        }

        self.total_bytes += data_len;
        self.entries.insert(
            key,
            CacheEntry {
                data,
                inserted_at: Instant::now(),
            },
        );

        if self.total_bytes > MAX_BYTES {
            self.evict_expired();
            self.evict_to_target();
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.total_bytes = 0;
    }

    fn evict_expired(&mut self) {
        let now = Instant::now();
        let expired: Vec<(i64, i32)> = self
            .entries
            .iter()
            .filter(|(_, v)| now.duration_since(v.inserted_at).as_secs() > TTL_SECS)
            .map(|(k, _)| *k)
            .collect();

        if expired.is_empty() {
            return;
        }

        let mut freed: u64 = 0;
        let count = expired.len();
        for key in expired {
            if let Some(entry) = self.entries.remove(&key) {
                freed += entry.data.len() as u64;
                self.total_bytes -= entry.data.len() as u64;
            }
        }

        tracing::info!(
            "[tg-cache] evicted {} expired entries, freed {} bytes, remaining {} bytes",
            count,
            freed,
            self.total_bytes
        );
    }

    fn evict_to_target(&mut self) {
        let target = MAX_BYTES * 3 / 4;
        if self.total_bytes <= target {
            return;
        }

        let mut by_age: Vec<(i64, i32)> = self.entries.keys().copied().collect();
        by_age.sort_by_key(|k| self.entries[k].inserted_at);

        let mut freed: u64 = 0;
        let mut count: usize = 0;
        for key in by_age {
            if self.total_bytes <= target {
                break;
            }
            if let Some(entry) = self.entries.remove(&key) {
                freed += entry.data.len() as u64;
                self.total_bytes -= entry.data.len() as u64;
                count += 1;
            }
        }

        tracing::info!(
            "[tg-cache] evicted {} entries for capacity, freed {} bytes, remaining {} bytes",
            count,
            freed,
            self.total_bytes
        );
    }
}

static CACHE: OnceLock<Mutex<ThumbnailCache>> = OnceLock::new();

fn cache() -> &'static Mutex<ThumbnailCache> {
    CACHE.get_or_init(|| Mutex::new(ThumbnailCache::new()))
}

fn extract_thumbnail_location(
    media: &tl::enums::MessageMedia,
) -> Option<(tl::enums::InputFileLocation, u64)> {
    match media {
        tl::enums::MessageMedia::Photo(photo_media) => {
            let photo = match photo_media.photo.as_ref()? {
                tl::enums::Photo::Photo(p) => p,
                tl::enums::Photo::Empty(_) => return None,
            };
            let smallest = photo
                .sizes
                .iter()
                .filter_map(|s| match s {
                    tl::enums::PhotoSize::Size(ps) => Some(ps),
                    _ => None,
                })
                .min_by_key(|ps| ps.size)?;

            let location = tl::enums::InputFileLocation::InputPhotoFileLocation(
                tl::types::InputPhotoFileLocation {
                    id: photo.id,
                    access_hash: photo.access_hash,
                    file_reference: photo.file_reference.clone(),
                    thumb_size: smallest.r#type.clone(),
                },
            );
            Some((location, smallest.size as u64))
        }
        tl::enums::MessageMedia::Document(doc_media) => {
            let doc = match doc_media.document.as_ref()? {
                tl::enums::Document::Document(d) => d,
                tl::enums::Document::Empty(_) => return None,
            };
            let thumbs = doc.thumbs.as_ref()?;
            let smallest = thumbs
                .iter()
                .filter_map(|s| match s {
                    tl::enums::PhotoSize::Size(ps) => Some(ps),
                    _ => None,
                })
                .min_by_key(|ps| ps.size)?;

            let location = tl::enums::InputFileLocation::InputDocumentFileLocation(
                tl::types::InputDocumentFileLocation {
                    id: doc.id,
                    access_hash: doc.access_hash,
                    file_reference: doc.file_reference.clone(),
                    thumb_size: smallest.r#type.clone(),
                },
            );
            Some((location, smallest.size as u64))
        }
        _ => None,
    }
}

async fn download_to_memory(
    client: &Client,
    location: tl::enums::InputFileLocation,
    total_size: u64,
) -> anyhow::Result<Vec<u8>> {
    use std::time::Duration;
    const PART_SIZE: i32 = 512 * 1024;
    const CHUNK_TIMEOUT_SECS: u64 = 15;
    const MAX_CHUNKS: usize = 256;
    const MAX_CHUNK_RETRIES: u32 = 3;
    let mut data = Vec::with_capacity(total_size as usize);
    let mut offset: i64 = 0;

    for _ in 0..MAX_CHUNKS {
        let request = tl::functions::upload::GetFile {
            precise: true,
            cdn_supported: false,
            location: location.clone(),
            offset,
            limit: PART_SIZE,
        };

        let mut last_err: Option<String> = None;
        let mut maybe_response: Option<tl::enums::upload::File> = None;
        for attempt in 0..MAX_CHUNK_RETRIES {
            let invoke_fut = client.invoke(&request);
            match tokio::time::timeout(Duration::from_secs(CHUNK_TIMEOUT_SECS), invoke_fut).await {
                Ok(Ok(r)) => {
                    maybe_response = Some(r);
                    break;
                }
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    if super::api::is_retryable_error(&msg) && attempt < MAX_CHUNK_RETRIES - 1 {
                        let backoff = (100_f64 * 1.1f64.powi(attempt as i32)) as u64;
                        let backoff = backoff.min(5000);
                        tracing::warn!(
                            "[tg-api] retryable error '{}' attempt={} backoff={}ms (download_to_memory)",
                            msg.chars().take(120).collect::<String>(),
                            attempt + 1,
                            backoff
                        );
                        tokio::time::sleep(Duration::from_millis(backoff)).await;
                        last_err = Some(msg);
                        continue;
                    }
                    return Err(anyhow::anyhow!("upload.GetFile thumbnail: {}", msg));
                }
                Err(_) => {
                    last_err = Some(format!("timeout after {}s", CHUNK_TIMEOUT_SECS));
                    if attempt < MAX_CHUNK_RETRIES - 1 {
                        tracing::warn!(
                            "[tg-api] chunk timeout {}s attempt={} (download_to_memory)",
                            CHUNK_TIMEOUT_SECS,
                            attempt + 1
                        );
                        continue;
                    }
                    return Err(anyhow::anyhow!(
                        "upload.GetFile timed out after {}s",
                        CHUNK_TIMEOUT_SECS
                    ));
                }
            }
        }
        let response = match maybe_response {
            Some(r) => r,
            None => {
                return Err(anyhow::anyhow!(
                    "upload.GetFile failed after retries: {}",
                    last_err.unwrap_or_default()
                ))
            }
        };

        let bytes = match response {
            tl::enums::upload::File::File(f) => f.bytes,
            tl::enums::upload::File::CdnRedirect(redirect) => {
                tracing::info!(
                    "[tg-cdn] redirect dc={}, downloading via CDN",
                    redirect.dc_id
                );
                let cdn_data = super::cdn::download_via_cdn(client, &redirect, total_size).await?;
                if cdn_data.len() > data.len() {
                    data.extend_from_slice(&cdn_data[data.len()..]);
                } else if !cdn_data.is_empty() && data.is_empty() {
                    data.extend_from_slice(&cdn_data);
                }
                return Ok(data);
            }
        };

        if bytes.is_empty() {
            break;
        }

        let len = bytes.len();
        data.extend_from_slice(&bytes);
        offset += len as i64;

        if (len as i32) < PART_SIZE {
            return Ok(data);
        }
    }

    Ok(data)
}

pub async fn get_thumbnail(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
) -> anyhow::Result<Vec<u8>> {
    let key = (chat_id, message_id);

    {
        let mut c = cache().lock().await;
        if let Some(data) = c.get(&key) {
            return Ok(data);
        }
    }

    if let Some(disk) = disk_cache() {
        let dk = disk_key(chat_id, message_id);
        if let Some(bytes) = disk.get(&dk).await {
            tracing::debug!(
                "[preview-cache] hit chat={}/{} ({}B)",
                chat_id,
                message_id,
                bytes.len()
            );
            let mut c = cache().lock().await;
            c.insert(key, bytes.clone());
            return Ok(bytes);
        }
    }

    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let mut access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let mut is_channel = chat_type == "channel" || (chat_type == "group" && access_hash != 0);

    let raw_media =
        match fetch_raw_media(&client, chat_id, access_hash, is_channel, message_id).await {
            Ok((media, _date)) => media,
            Err(e)
                if chat_type == "channel"
                    && super::api::is_channel_invalid_error(&e.to_string()) =>
            {
                tracing::warn!(
                "[tg-thumb] CHANNEL_INVALID for chat={}/{} (hash={}), refreshing hash and retrying",
                chat_id, message_id, access_hash
            );
                match super::api::refresh_channel_hash(handle, chat_id).await {
                    Ok(Some(new_hash)) if new_hash != access_hash => {
                        access_hash = new_hash;
                        is_channel = true;
                        let (media, _date) =
                            fetch_raw_media(&client, chat_id, access_hash, is_channel, message_id)
                                .await?;
                        media
                    }
                    _ => return Err(e),
                }
            }
            Err(e) => return Err(e),
        };

    let (location, total_size) = extract_thumbnail_location(&raw_media)
        .ok_or_else(|| anyhow::anyhow!("No thumbnail available"))?;

    let data = download_to_memory(&client, location, total_size).await?;

    {
        let mut c = cache().lock().await;
        c.insert(key, data.clone());
    }

    if data.len() > DISK_CACHE_THRESHOLD_BYTES {
        if let Some(disk) = disk_cache() {
            let dk = disk_key(chat_id, message_id);
            let bytes = data.clone();
            tokio::spawn(async move {
                if let Err(e) = disk.put(&dk, &bytes).await {
                    tracing::warn!("[preview-cache] put failed: {}", e);
                }
            });
        }
    }

    Ok(data)
}

async fn fetch_chat_photo_id(
    client: &Client,
    chat_id: i64,
    chat_type: &str,
    access_hash: i64,
) -> anyhow::Result<i64> {
    match chat_type {
        "private" => {
            let request = tl::functions::users::GetUsers {
                id: vec![tl::enums::InputUser::User(tl::types::InputUser {
                    user_id: chat_id,
                    access_hash,
                })],
            };
            let users = client
                .invoke(&request)
                .await
                .map_err(|e| anyhow::anyhow!("users.GetUsers: {}", e))?;
            for user in users {
                if let tl::enums::User::User(u) = user {
                    if u.id == chat_id {
                        if let Some(tl::enums::UserProfilePhoto::Photo(photo)) = u.photo {
                            return Ok(photo.photo_id);
                        }
                    }
                }
            }
            Err(anyhow::anyhow!("User has no photo"))
        }
        _ => {
            if access_hash != 0 {
                let request = tl::functions::channels::GetChannels {
                    id: vec![tl::enums::InputChannel::Channel(tl::types::InputChannel {
                        channel_id: chat_id,
                        access_hash,
                    })],
                };
                let result = client
                    .invoke(&request)
                    .await
                    .map_err(|e| anyhow::anyhow!("channels.GetChannels: {}", e))?;
                let chats = match result {
                    tl::enums::messages::Chats::Chats(c) => c.chats,
                    tl::enums::messages::Chats::Slice(c) => c.chats,
                };
                for chat in chats {
                    match chat {
                        tl::enums::Chat::Channel(c) if c.id == chat_id => {
                            if let tl::enums::ChatPhoto::Photo(photo) = c.photo {
                                return Ok(photo.photo_id);
                            }
                        }
                        _ => {}
                    }
                }
                Err(anyhow::anyhow!("Channel has no photo"))
            } else {
                let request = tl::functions::messages::GetChats { id: vec![chat_id] };
                let result = client
                    .invoke(&request)
                    .await
                    .map_err(|e| anyhow::anyhow!("messages.GetChats: {}", e))?;
                let chats = match result {
                    tl::enums::messages::Chats::Chats(c) => c.chats,
                    tl::enums::messages::Chats::Slice(c) => c.chats,
                };
                for chat in chats {
                    if let tl::enums::Chat::Chat(c) = chat {
                        if c.id == chat_id {
                            if let tl::enums::ChatPhoto::Photo(photo) = c.photo {
                                return Ok(photo.photo_id);
                            }
                        }
                    }
                }
                Err(anyhow::anyhow!("Chat has no photo"))
            }
        }
    }
}

pub async fn get_chat_photo(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
) -> anyhow::Result<Vec<u8>> {
    let key = (chat_id, 0i32);

    {
        let mut c = cache().lock().await;
        if let Some(data) = c.get(&key) {
            return Ok(data);
        }
    }

    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    let cached_photo_id = guard.peer_photo_ids.get(&chat_id).copied();
    drop(guard);

    let input_peer = super::api::make_input_peer(chat_id, chat_type, access_hash);
    let photo_id = match cached_photo_id {
        Some(p) => p,
        None => fetch_chat_photo_id(&client, chat_id, chat_type, access_hash).await?,
    };

    let location = tl::enums::InputFileLocation::InputPeerPhotoFileLocation(
        tl::types::InputPeerPhotoFileLocation {
            big: false,
            peer: input_peer,
            photo_id,
        },
    );

    let data = download_to_memory(&client, location, 0).await?;

    {
        let mut c = cache().lock().await;
        c.insert(key, data.clone());
    }

    Ok(data)
}

pub async fn clear_cache() {
    cache().lock().await.clear();
}
