use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Instant;

use grammers_client::Client;
use grammers_client::grammers_tl_types as tl;
use tokio::sync::Mutex;

use super::auth::TelegramSessionHandle;
use super::parallel_download::fetch_raw_media;

const TTL_SECS: u64 = 120;
const MAX_BYTES: u64 = 50 * 1024 * 1024;

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
        self.entries.insert(key, CacheEntry {
            data,
            inserted_at: Instant::now(),
        });

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

        tracing::debug!(
            "[tg-cache] evicted {} expired entries, freed {} bytes",
            count,
            freed
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

        tracing::debug!(
            "[tg-cache] evicted {} entries for capacity, freed {} bytes",
            count,
            freed
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
    const PART_SIZE: i32 = 512 * 1024;
    let mut data = Vec::with_capacity(total_size as usize);
    let mut offset: i64 = 0;

    loop {
        let request = tl::functions::upload::GetFile {
            precise: true,
            cdn_supported: false,
            location: location.clone(),
            offset,
            limit: PART_SIZE,
        };

        let response = client
            .invoke(&request)
            .await
            .map_err(|e| anyhow::anyhow!("upload.GetFile thumbnail: {}", e))?;

        let bytes = match response {
            tl::enums::upload::File::File(f) => f.bytes,
            tl::enums::upload::File::CdnRedirect(_) => {
                return Err(anyhow::anyhow!("CDN redirect not supported"));
            }
        };

        if bytes.is_empty() {
            break;
        }

        let len = bytes.len();
        data.extend_from_slice(&bytes);
        offset += len as i64;

        if (len as i32) < PART_SIZE {
            break;
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

    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?
        .clone();
    let access_hash = guard.peer_hashes.get(&chat_id).copied().unwrap_or(0);
    drop(guard);

    let is_channel = chat_type == "channel" || (chat_type == "group" && access_hash != 0);

    let (raw_media, _date) =
        fetch_raw_media(&client, chat_id, access_hash, is_channel, message_id).await?;

    let (location, total_size) = extract_thumbnail_location(&raw_media)
        .ok_or_else(|| anyhow::anyhow!("No thumbnail available"))?;

    let data = download_to_memory(&client, location, total_size).await?;

    {
        let mut c = cache().lock().await;
        c.insert(key, data.clone());
    }

    Ok(data)
}

pub async fn clear_cache() {
    cache().lock().await.clear();
}
