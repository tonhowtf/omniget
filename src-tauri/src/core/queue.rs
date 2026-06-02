use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

static EMIT_COUNT: AtomicU64 = AtomicU64::new(0);

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn append_download_log(app: &tauri::AppHandle, id: u64, line: impl AsRef<str>) {
    crate::core::download_log::push_line(id, line.as_ref());
    let _ = app.emit(
        "download-log-update",
        serde_json::json!({
            "id": id,
        }),
    );
}

use serde::Serialize;
use tauri::{Emitter, Manager};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn shared_http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .build()
            .unwrap_or_default()
    })
}

use crate::core::ffmpeg::{self, MetadataEmbed};
use crate::models::media::MediaInfo;
use crate::platforms::traits::PlatformDownloader;
use crate::storage::config;

struct CachedInfo {
    info: MediaInfo,
    cached_at: std::time::Instant,
}

static INFO_CACHE: OnceLock<tokio::sync::Mutex<HashMap<String, CachedInfo>>> = OnceLock::new();

fn info_cache() -> &'static tokio::sync::Mutex<HashMap<String, CachedInfo>> {
    INFO_CACHE.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

const INFO_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(600);

static IN_FLIGHT_FETCHES: OnceLock<
    tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
> = OnceLock::new();

fn in_flight_map() -> &'static tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>> {
    IN_FLIGHT_FETCHES.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaPreviewEvent {
    pub url: String,
    pub title: String,
    pub author: String,
    pub thumbnail_url: Option<String>,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueKind {
    Video,
    Audio,
    Image,
    Pdf,
    Book,
    Webpage,
    TelegramMedia,
    CourseLesson,
    Generic,
}

pub fn kind_from_platform(platform: &str) -> QueueKind {
    let p = platform.to_ascii_lowercase();
    match p.as_str() {
        "youtube" | "vimeo" | "twitch" | "bilibili" | "tiktok" | "twitter" | "x" | "instagram"
        | "reddit" | "bluesky" | "facebook" | "generic_ytdlp" => QueueKind::Video,
        "soundcloud" | "spotify" => QueueKind::Audio,
        "pinterest" => QueueKind::Image,
        "magnet" | "p2p" | "torrent" => QueueKind::Generic,
        "telegram" | "telegram_media" => QueueKind::TelegramMedia,
        "courses" | "course_lesson" => QueueKind::CourseLesson,
        "annas_archive" | "book" | "libgen" | "gutendex" => QueueKind::Book,
        "pdf" => QueueKind::Pdf,
        "webpage" | "embed" => QueueKind::Webpage,
        _ => QueueKind::Generic,
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum QueueStatus {
    Queued,
    Active,
    Paused,
    Seeding,
    Complete { success: bool },
    Error { message: String, retryable: bool },
}

pub fn is_retryable_error_message(message: &str) -> bool {
    let lower = message.to_lowercase();
    if lower.contains("cancel") {
        return false;
    }
    let (category, _) = omniget_core::core::errors::classify_download_error(message);
    matches!(category, "unknown" | "rate_limited")
}

#[derive(Clone, Serialize)]
pub struct QueueItemInfo {
    pub id: u64,
    pub url: String,
    pub platform: String,
    pub title: String,
    pub status: QueueStatus,
    pub percent: f64,
    pub speed_bytes_per_sec: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub file_path: Option<String>,
    pub file_size_bytes: Option<u64>,
    pub file_count: Option<u32>,
    pub thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<QueueKind>,
    #[serde(default)]
    pub external: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_mode: Option<String>,
}

pub struct QueueItem {
    pub id: u64,
    pub url: String,
    pub platform: String,
    pub title: String,
    pub status: QueueStatus,
    pub cancel_token: CancellationToken,
    pub output_dir: String,
    pub download_mode: Option<String>,
    pub quality: Option<String>,
    pub format_id: Option<String>,
    pub referer: Option<String>,
    pub extra_headers: Option<std::collections::HashMap<String, String>>,
    pub page_url: Option<String>,
    pub user_agent: Option<String>,
    pub percent: f64,
    pub speed_bytes_per_sec: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub file_path: Option<String>,
    pub file_size_bytes: Option<u64>,
    pub file_count: Option<u32>,
    pub media_info: Option<MediaInfo>,
    pub downloader: Arc<dyn PlatformDownloader>,
    pub ytdlp_path: Option<PathBuf>,
    pub from_hotkey: bool,
    pub torrent_id: Option<usize>,
    pub kind: Option<QueueKind>,
    pub external: bool,
    pub thumbnail_url_override: Option<String>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub resume_state: Option<serde_json::Value>,
    pub concurrent_segments: Option<usize>,
    pub segment_size_bytes: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub cookie_slug: Option<String>,
    pub custom_ytdlp_args: Option<Vec<String>>,
    pub torrent_files: Option<Vec<usize>>,
    pub scheduled_at_ms: Option<u64>,
    pub stop_at_ms: Option<u64>,
}

impl QueueItem {
    pub fn to_info(&self) -> QueueItemInfo {
        QueueItemInfo {
            id: self.id,
            url: self.url.clone(),
            platform: self.platform.clone(),
            title: self.title.clone(),
            status: self.status.clone(),
            percent: self.percent,
            speed_bytes_per_sec: self.speed_bytes_per_sec,
            downloaded_bytes: self.downloaded_bytes,
            total_bytes: self.total_bytes,
            file_path: self.file_path.clone(),
            file_size_bytes: self.file_size_bytes,
            file_count: self.file_count,
            thumbnail_url: self.thumbnail_url_override.clone().or_else(|| {
                self.media_info
                    .as_ref()
                    .and_then(|m| m.thumbnail_url.clone())
            }),
            kind: self.kind,
            external: self.external,
            eta_seconds: self.eta_seconds,
            quality: self.quality.clone(),
            download_mode: self.download_mode.clone(),
        }
    }
}

pub struct DownloadQueue {
    pub items: Vec<QueueItem>,
    pub max_concurrent: u32,
    pub stagger_delay_ms: u64,
    pub default_max_retries: u32,
}

impl DownloadQueue {
    pub fn new(max_concurrent: u32) -> Self {
        Self {
            items: Vec::new(),
            max_concurrent,
            stagger_delay_ms: 150,
            default_max_retries: 3,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn enqueue(
        &mut self,
        id: u64,
        url: String,
        platform: String,
        title: String,
        output_dir: String,
        download_mode: Option<String>,
        quality: Option<String>,
        format_id: Option<String>,
        referer: Option<String>,
        extra_headers: Option<std::collections::HashMap<String, String>>,
        page_url: Option<String>,
        user_agent: Option<String>,
        media_info: Option<MediaInfo>,
        total_bytes: Option<u64>,
        file_count: Option<u32>,
        downloader: Arc<dyn PlatformDownloader>,
        ytdlp_path: Option<PathBuf>,
        from_hotkey: bool,
        cookie_slug: Option<String>,
        custom_ytdlp_args: Option<Vec<String>>,
        torrent_files: Option<Vec<usize>>,
        scheduled_at_ms: Option<u64>,
        stop_at_ms: Option<u64>,
    ) {
        let computed_kind = Some(kind_from_platform(&platform));
        let item = QueueItem {
            id,
            url,
            platform,
            title,
            status: QueueStatus::Queued,
            cancel_token: CancellationToken::new(),
            output_dir,
            download_mode,
            quality,
            format_id,
            referer,
            extra_headers,
            page_url,
            user_agent,
            percent: 0.0,
            speed_bytes_per_sec: 0.0,
            downloaded_bytes: 0,
            total_bytes,
            file_path: None,
            file_size_bytes: None,
            file_count,
            media_info,
            downloader,
            ytdlp_path,
            from_hotkey,
            torrent_id: None,
            kind: computed_kind,
            external: false,
            thumbnail_url_override: None,
            retry_count: 0,
            max_retries: self.default_max_retries,
            resume_state: None,
            concurrent_segments: None,
            segment_size_bytes: None,
            eta_seconds: None,
            cookie_slug,
            custom_ytdlp_args,
            torrent_files,
            scheduled_at_ms,
            stop_at_ms,
        };
        crate::core::recovery::persist(crate::core::recovery::RecoveryItem {
            id: item.id,
            url: item.url.clone(),
            title: item.title.clone(),
            platform: item.platform.clone(),
            output_dir: item.output_dir.clone(),
            download_mode: item.download_mode.clone(),
            quality: item.quality.clone(),
            format_id: item.format_id.clone(),
            referer: item.referer.clone(),
        });
        self.items.push(item);
    }

    pub fn hydrate_from_history(&mut self) {
        let entries = crate::core::queue_history::list();
        if entries.is_empty() {
            return;
        }
        let placeholder: Arc<dyn PlatformDownloader> =
            Arc::new(crate::platforms::noop::NoopDownloader::new());
        for entry in entries.iter().rev() {
            if self.items.iter().any(|i| i.id == entry.id) {
                continue;
            }
            let status = if entry.success {
                QueueStatus::Complete { success: true }
            } else {
                let msg = entry.error.clone().unwrap_or_default();
                let retryable = is_retryable_error_message(&msg);
                QueueStatus::Error {
                    message: msg,
                    retryable,
                }
            };
            let percent = if entry.success { 100.0 } else { 0.0 };
            let item = QueueItem {
                id: entry.id,
                url: entry.url.clone(),
                platform: entry.platform.clone(),
                title: entry.title.clone(),
                status,
                cancel_token: CancellationToken::new(),
                output_dir: entry
                    .file_path
                    .as_ref()
                    .and_then(|p| {
                        std::path::Path::new(p)
                            .parent()
                            .map(|x| x.to_string_lossy().to_string())
                    })
                    .unwrap_or_default(),
                download_mode: None,
                quality: None,
                format_id: None,
                referer: None,
                extra_headers: None,
                page_url: None,
                user_agent: None,
                percent,
                speed_bytes_per_sec: 0.0,
                downloaded_bytes: entry.file_size_bytes.unwrap_or(0),
                total_bytes: entry.total_bytes,
                file_path: entry.file_path.clone(),
                file_size_bytes: entry.file_size_bytes,
                file_count: None,
                media_info: None,
                downloader: placeholder.clone(),
                ytdlp_path: None,
                from_hotkey: false,
                torrent_id: None,
                kind: entry.kind,
                external: false,
                thumbnail_url_override: entry.thumbnail_url.clone(),
                retry_count: 0,
                max_retries: 0,
                resume_state: None,
                concurrent_segments: None,
                segment_size_bytes: None,
                eta_seconds: None,
                cookie_slug: None,
                custom_ytdlp_args: None,
                torrent_files: None,
                scheduled_at_ms: None,
                stop_at_ms: None,
            };
            self.items.push(item);
        }
    }

    pub fn active_count(&self) -> u32 {
        self.items
            .iter()
            .filter(|i| i.status == QueueStatus::Active)
            .count() as u32
    }

    pub fn next_queued_ids(&self) -> Vec<u64> {
        let slots = self.max_concurrent.saturating_sub(self.active_count()) as usize;
        let now = now_ms();
        self.items
            .iter()
            .filter(|i| i.status == QueueStatus::Queued)
            .filter(|i| i.scheduled_at_ms.map(|t| now >= t).unwrap_or(true))
            .take(slots)
            .map(|i| i.id)
            .collect()
    }

    pub fn next_available_id(&self, preferred: u64) -> u64 {
        let mut id = preferred;
        while self.items.iter().any(|i| i.id == id) {
            id = id.saturating_add(1);
        }
        id
    }

    pub fn mark_active(&mut self, id: u64) {
        if let Some(item) = self
            .items
            .iter_mut()
            .find(|i| i.id == id && i.status == QueueStatus::Queued)
        {
            item.status = QueueStatus::Active;
            item.cancel_token = CancellationToken::new();
        }
    }

    pub fn mark_complete(
        &mut self,
        id: u64,
        success: bool,
        error: Option<String>,
        file_path: Option<String>,
        file_size_bytes: Option<u64>,
    ) {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            let error_for_history = error.clone();
            if success {
                item.status = QueueStatus::Complete { success: true };
                item.percent = 100.0;
            } else {
                let msg = error.unwrap_or_default();
                let retryable = is_retryable_error_message(&msg);
                item.status = QueueStatus::Error {
                    message: msg,
                    retryable,
                };
            }
            item.file_path = file_path;
            item.file_size_bytes = file_size_bytes;
            item.speed_bytes_per_sec = 0.0;
            item.eta_seconds = None;
            crate::core::recovery::remove(id);

            if !item.external {
                let entry = crate::core::queue_history::HistoryEntry {
                    id: item.id,
                    url: item.url.clone(),
                    platform: item.platform.clone(),
                    title: item.title.clone(),
                    file_path: item.file_path.clone(),
                    file_size_bytes: item.file_size_bytes,
                    total_bytes: item.total_bytes,
                    success,
                    error: if success { None } else { error_for_history },
                    completed_at: crate::core::queue_history::now_unix_seconds(),
                    thumbnail_url: item.thumbnail_url_override.clone().or_else(|| {
                        item.media_info
                            .as_ref()
                            .and_then(|m| m.thumbnail_url.clone())
                    }),
                    kind: item.kind,
                };
                crate::core::queue_history::record(entry);
            }
        }
    }

    pub fn mark_seeding(
        &mut self,
        id: u64,
        file_path: Option<String>,
        file_size_bytes: Option<u64>,
        torrent_id: Option<usize>,
    ) {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            item.status = QueueStatus::Seeding;
            item.percent = 100.0;
            item.file_path = file_path;
            item.file_size_bytes = file_size_bytes;
            item.speed_bytes_per_sec = 0.0;
            item.torrent_id = torrent_id;
            crate::core::recovery::remove(id);
        }
    }

    pub fn update_progress(
        &mut self,
        id: u64,
        percent: f64,
        speed: f64,
        downloaded: u64,
        total: Option<u64>,
        torrent_id: Option<usize>,
        eta_seconds: Option<u64>,
    ) {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            if item.status != QueueStatus::Active {
                if torrent_id.is_some() && item.torrent_id.is_none() {
                    item.torrent_id = torrent_id;
                }
                return;
            }
            item.percent = percent;
            item.speed_bytes_per_sec = speed;
            item.downloaded_bytes = downloaded;
            if let Some(t) = total {
                item.total_bytes = Some(t);
            }
            if torrent_id.is_some() && item.torrent_id.is_none() {
                item.torrent_id = torrent_id;
            }
            item.eta_seconds = eta_seconds;
        }
    }

    pub fn pause(&mut self, id: u64) -> bool {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            if item.status == QueueStatus::Active {
                if item.platform != "magnet"
                    && !omniget_core::core::ytdlp::pause_download_process(id)
                {
                    return false;
                }
                item.status = QueueStatus::Paused;
                item.speed_bytes_per_sec = 0.0;
                item.eta_seconds = None;
                return true;
            }
        }
        false
    }

    pub fn resume(&mut self, id: u64) -> bool {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            if item.status == QueueStatus::Paused {
                if item.platform != "magnet"
                    && !omniget_core::core::ytdlp::resume_download_process(id)
                {
                    return false;
                }
                item.status = QueueStatus::Active;
                return true;
            }
        }
        false
    }

    pub fn pause_all(&mut self) -> Vec<(u64, Option<usize>)> {
        let mut paused = Vec::new();
        for item in self.items.iter_mut() {
            if item.status == QueueStatus::Active {
                if item.platform != "magnet"
                    && !omniget_core::core::ytdlp::pause_download_process(item.id)
                {
                    continue;
                }
                item.status = QueueStatus::Paused;
                item.speed_bytes_per_sec = 0.0;
                item.eta_seconds = None;
                paused.push((item.id, item.torrent_id));
            }
        }
        paused
    }

    pub fn resume_all(&mut self) -> Vec<(u64, Option<usize>)> {
        let mut resumed = Vec::new();
        for item in self.items.iter_mut() {
            if item.status == QueueStatus::Paused {
                let tid = item.torrent_id;
                if item.platform != "magnet"
                    && !omniget_core::core::ytdlp::resume_download_process(item.id)
                {
                    continue;
                }
                item.status = QueueStatus::Active;
                resumed.push((item.id, tid));
            }
        }
        resumed
    }

    pub fn reorder(&mut self, ids_in_order: Vec<u64>) -> bool {
        let mut slots: Vec<Option<QueueItem>> = self.items.drain(..).map(Some).collect();

        let queued_slot_indices: Vec<usize> = slots
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| {
                slot.as_ref()
                    .filter(|i| i.status == QueueStatus::Queued)
                    .map(|_| idx)
            })
            .collect();

        if queued_slot_indices.is_empty() {
            self.items = slots.into_iter().flatten().collect();
            return false;
        }

        let queued_id_to_slot: std::collections::HashMap<u64, usize> = queued_slot_indices
            .iter()
            .map(|idx| (slots[*idx].as_ref().unwrap().id, *idx))
            .collect();

        let mut new_queued_order: Vec<QueueItem> = Vec::with_capacity(queued_slot_indices.len());
        let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();

        for id in &ids_in_order {
            if seen.contains(id) {
                continue;
            }
            if let Some(slot_idx) = queued_id_to_slot.get(id) {
                if let Some(item) = slots[*slot_idx].take() {
                    new_queued_order.push(item);
                    seen.insert(*id);
                }
            }
        }
        for idx in &queued_slot_indices {
            if let Some(item) = slots[*idx].take() {
                new_queued_order.push(item);
            }
        }

        let mut iter = new_queued_order.into_iter();
        let mut rebuilt: Vec<QueueItem> = Vec::with_capacity(slots.len());
        for (idx, slot) in slots.into_iter().enumerate() {
            if queued_slot_indices.contains(&idx) {
                if let Some(item) = iter.next() {
                    rebuilt.push(item);
                }
            } else if let Some(item) = slot {
                rebuilt.push(item);
            }
        }
        rebuilt.extend(iter);
        self.items = rebuilt;
        true
    }

    /// Cancel an item. Returns the torrent_id if the item needs torrent cleanup (caller should delete from session).
    pub fn cancel(&mut self, id: u64) -> (bool, Option<usize>) {
        let result = self.cancel_inner(id);
        if result.0 {
            crate::core::recovery::remove(id);
        }
        result
    }

    fn cancel_inner(&mut self, id: u64) -> (bool, Option<usize>) {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            match &item.status {
                QueueStatus::Active => {
                    item.cancel_token.cancel();
                    item.status = QueueStatus::Error {
                        message: "Cancelled".to_string(),
                        retryable: false,
                    };
                    item.speed_bytes_per_sec = 0.0;
                    return (true, None);
                }
                QueueStatus::Seeding => {
                    let tid = item.torrent_id;
                    item.status = QueueStatus::Error {
                        message: "Cancelled".to_string(),
                        retryable: false,
                    };
                    item.speed_bytes_per_sec = 0.0;
                    return (true, tid);
                }
                QueueStatus::Paused => {
                    // For magnet downloads, the cancel_token was not cancelled during pause,
                    // so we must cancel it now to stop the background download loop.
                    // Also return the torrent_id for session cleanup.
                    item.cancel_token.cancel();
                    let tid = if item.platform == "magnet" {
                        item.torrent_id
                    } else {
                        None
                    };
                    item.status = QueueStatus::Error {
                        message: "Cancelled".to_string(),
                        retryable: false,
                    };
                    item.speed_bytes_per_sec = 0.0;
                    return (true, tid);
                }
                QueueStatus::Queued => {
                    item.status = QueueStatus::Error {
                        message: "Cancelled".to_string(),
                        retryable: false,
                    };
                    return (true, None);
                }
                _ => {}
            }
        }
        (false, None)
    }

    pub fn retry(&mut self, id: u64) -> bool {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            if matches!(item.status, QueueStatus::Error { .. }) {
                item.status = QueueStatus::Queued;
                item.cancel_token = CancellationToken::new();
                item.percent = 0.0;
                item.speed_bytes_per_sec = 0.0;
                item.downloaded_bytes = 0;
                item.file_path = None;
                item.file_size_bytes = None;
                item.retry_count = 0;
                return true;
            }
        }
        false
    }

    /// Remove an item. Returns the torrent_id if the item needs torrent cleanup (caller should delete from session).
    pub fn remove(&mut self, id: u64) -> Option<Option<usize>> {
        let result = self.remove_inner(id);
        if result.is_some() {
            crate::core::recovery::remove(id);
            crate::core::queue_history::remove(id);
        }
        result
    }

    fn remove_inner(&mut self, id: u64) -> Option<Option<usize>> {
        if let Some(pos) = self.items.iter().position(|i| i.id == id) {
            let item = &self.items[pos];
            if item.status == QueueStatus::Active {
                item.cancel_token.cancel();
            }
            // For paused magnet items, the cancel_token was not cancelled during pause
            if item.status == QueueStatus::Paused && item.platform == "magnet" {
                item.cancel_token.cancel();
            }
            let torrent_id = if item.status == QueueStatus::Seeding
                || (item.status == QueueStatus::Paused && item.platform == "magnet")
            {
                item.torrent_id
            } else {
                None
            };
            self.items.remove(pos);
            return Some(torrent_id);
        }
        None
    }

    pub fn clear_finished(&mut self) {
        let to_remove: Vec<u64> = self
            .items
            .iter()
            .filter(|i| {
                matches!(
                    i.status,
                    QueueStatus::Complete { .. } | QueueStatus::Error { .. }
                )
            })
            .map(|i| i.id)
            .collect();
        for id in &to_remove {
            crate::core::recovery::remove(*id);
            crate::core::queue_history::remove(*id);
        }
        self.items.retain(|i| {
            !matches!(
                i.status,
                QueueStatus::Complete { .. } | QueueStatus::Error { .. }
            )
        });
    }

    pub fn get_state(&self) -> Vec<QueueItemInfo> {
        self.items.iter().map(|i| i.to_info()).collect()
    }

    pub fn has_url(&self, url: &str) -> bool {
        self.items.iter().any(|i| {
            i.url == url
                && matches!(
                    i.status,
                    QueueStatus::Queued
                        | QueueStatus::Active
                        | QueueStatus::Paused
                        | QueueStatus::Seeding
                )
        })
    }
}

pub struct ProgressThrottle {
    last_emit: std::time::Instant,
    min_interval: std::time::Duration,
}

impl ProgressThrottle {
    pub fn new(min_interval_ms: u64) -> Self {
        Self {
            last_emit: std::time::Instant::now() - std::time::Duration::from_secs(10),
            min_interval: std::time::Duration::from_millis(min_interval_ms),
        }
    }

    pub fn should_emit(&mut self) -> bool {
        let now = std::time::Instant::now();
        if now.duration_since(self.last_emit) >= self.min_interval {
            self.last_emit = now;
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Serialize)]
pub struct QueueItemProgress {
    pub id: u64,
    pub title: String,
    pub platform: String,
    pub percent: f64,
    pub speed_bytes_per_sec: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<u64>,
}

pub fn emit_queue_state_from_state(app: &tauri::AppHandle, state: Vec<QueueItemInfo>) {
    let n = EMIT_COUNT.fetch_add(1, Ordering::Relaxed);
    if n.is_multiple_of(10) {
        tracing::debug!("[perf] emit_queue_state called {} times", n);
    }
    let _ = app.emit("queue-state-update", &state);
    let total = crate::tray::compute_total_active(app);
    crate::tray::update_active_count(app, total);
    crate::core::awake::sync(total > 0);

    let active_items: Vec<_> = state
        .iter()
        .filter(|i| i.status == QueueStatus::Active)
        .collect();
    let avg_percent = if !active_items.is_empty() {
        let sum: f64 = active_items.iter().map(|i| i.percent).sum();
        sum / active_items.len() as f64 / 100.0
    } else {
        0.0
    };
    let total_speed: f64 = active_items.iter().map(|i| i.speed_bytes_per_sec).sum();
    crate::tray::update_speed_tooltip(app, total, total_speed);
    crate::tray::update_taskbar_badge(app, total, avg_percent);

    if let Some(window) = app.get_webview_window("main") {
        let title = if total > 0 {
            format!("({}) omniget", total)
        } else {
            "omniget".into()
        };
        let _ = window.set_title(&title);
    }
}

pub fn emit_queue_state(app: &tauri::AppHandle, queue: &DownloadQueue) {
    let state = queue.get_state();
    emit_queue_state_from_state(app, state);
}

/// RAII guard that ensures an Active queue item never leaks a slot.
///
/// If the download future panics or is dropped before reaching `mark_complete`
/// / `mark_seeding`, the Drop impl spawns a task that transitions the item to
/// Error("Download interrupted") and calls `try_start_next`, unblocking the
/// queue.
///
/// When the download reaches a terminal state through the normal paths, the
/// guard sees the item is no longer Active and does nothing (idempotent).
struct ActiveJobSlot {
    app: tauri::AppHandle,
    queue: Arc<tokio::sync::Mutex<DownloadQueue>>,
    item_id: u64,
    armed: bool,
}

impl ActiveJobSlot {
    fn new(
        app: tauri::AppHandle,
        queue: Arc<tokio::sync::Mutex<DownloadQueue>>,
        item_id: u64,
    ) -> Self {
        Self {
            app,
            queue,
            item_id,
            armed: true,
        }
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for ActiveJobSlot {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let app = self.app.clone();
        let queue = self.queue.clone();
        let item_id = self.item_id;
        tokio::spawn(async move {
            let state = {
                let mut q = queue.lock().await;
                let still_active = q
                    .items
                    .iter()
                    .find(|i| i.id == item_id)
                    .map(|i| i.status == QueueStatus::Active)
                    .unwrap_or(false);
                if !still_active {
                    return;
                }
                tracing::warn!(
                    "[queue] ActiveJobSlot guard firing for {} — download ended without clean release",
                    item_id
                );
                q.mark_complete(
                    item_id,
                    false,
                    Some("Download interrupted".to_string()),
                    None,
                    None,
                );
                q.get_state()
            };
            emit_queue_state_from_state(&app, state);
            try_start_next(app, queue).await;
        });
    }
}

pub fn spawn_download(
    app: tauri::AppHandle,
    queue: Arc<tokio::sync::Mutex<DownloadQueue>>,
    item_id: u64,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        let _timer_start = std::time::Instant::now();
        let slot = ActiveJobSlot::new(app.clone(), queue.clone(), item_id);
        spawn_download_inner(app, queue, item_id).await;
        slot.disarm();
        tracing::debug!(
            "[perf] spawn_download {} took {:?}",
            item_id,
            _timer_start.elapsed()
        );
    })
}

async fn spawn_download_inner(
    app: tauri::AppHandle,
    queue: Arc<tokio::sync::Mutex<DownloadQueue>>,
    item_id: u64,
) {
    tracing::info!("[queue] download {} started", item_id);

    let _ = app.emit(
        "queue-item-progress",
        &QueueItemProgress {
            id: item_id,
            title: "".to_string(),
            platform: "".to_string(),
            percent: 0.0,
            speed_bytes_per_sec: 0.0,
            downloaded_bytes: 0,
            total_bytes: None,
            phase: "preparing".to_string(),
            eta_seconds: None,
        },
    );

    let host_key = {
        let q = queue.lock().await;
        q.items
            .iter()
            .find(|i| i.id == item_id)
            .map(|i| crate::core::host_limiter::host_key_for_url(&i.url))
    };
    let _host_lease = match host_key {
        Some(key) => Some(crate::core::host_limiter::acquire(&key).await),
        None => None,
    };

    let (
        url,
        output_dir,
        download_mode,
        quality,
        format_id,
        referer,
        extra_headers,
        page_url,
        user_agent,
        cancel_token,
        media_info,
        platform_name,
        downloader,
        ytdlp_path,
        from_hotkey,
        cookie_slug,
        custom_ytdlp_args,
        torrent_files,
    ) = {
        let q = queue.lock().await;
        let item = match q.items.iter().find(|i| i.id == item_id) {
            Some(i) => i,
            None => return,
        };
        (
            item.url.clone(),
            item.output_dir.clone(),
            item.download_mode.clone(),
            item.quality.clone(),
            item.format_id.clone(),
            item.referer.clone(),
            item.extra_headers.clone(),
            item.page_url.clone(),
            item.user_agent.clone(),
            item.cancel_token.clone(),
            item.media_info.clone(),
            item.platform.clone(),
            item.downloader.clone(),
            item.ytdlp_path.clone(),
            item.from_hotkey,
            item.cookie_slug.clone(),
            item.custom_ytdlp_args.clone(),
            item.torrent_files.clone(),
        )
    };

    let info_start = std::time::Instant::now();
    let info = match media_info {
        Some(i) if !i.available_qualities.is_empty() => {
            tracing::info!(
                "[queue] info for {} from cache/pre-fetched in {:?}",
                item_id,
                info_start.elapsed()
            );
            i
        }
        _ => {
            tracing::debug!(
                "[perf] spawn_download_inner {}: media_info is None, fetching info",
                item_id
            );
            let _ = app.emit(
                "queue-item-progress",
                &QueueItemProgress {
                    id: item_id,
                    title: url.clone(),
                    platform: platform_name.clone(),
                    percent: 0.0,
                    speed_bytes_per_sec: 0.0,
                    downloaded_bytes: 0,
                    total_bytes: None,
                    phase: "fetching_info".to_string(),
                    eta_seconds: None,
                },
            );

            let info_result = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                fetch_and_cache_info(&url, &*downloader, &platform_name, ytdlp_path.as_deref()),
            )
            .await;

            match info_result {
                Ok(Ok(i)) => i,
                Ok(Err(e)) => {
                    let state = {
                        let mut q = queue.lock().await;
                        q.mark_complete(item_id, false, Some(e.to_string()), None, None);
                        q.get_state()
                    };
                    emit_queue_state_from_state(&app, state);
                    try_start_next(app, queue).await;
                    return;
                }
                Err(_) => {
                    tracing::warn!("[queue] info fetch timed out for {} after 60s", item_id);
                    let state = {
                        let mut q = queue.lock().await;
                        q.mark_complete(
                            item_id,
                            false,
                            Some("Timed out fetching video info".to_string()),
                            None,
                            None,
                        );
                        q.get_state()
                    };
                    emit_queue_state_from_state(&app, state);
                    try_start_next(app, queue).await;
                    return;
                }
            }
        }
    };
    tracing::info!(
        "[queue] info fetch for {} took {:?}",
        item_id,
        info_start.elapsed()
    );

    let mut info = info;
    if is_generic_title(&info.title) {
        let pokemon = omniget_core::core::pokemon_names::random_pokemon_name();
        info.title = format!("video_{}", pokemon);
    }

    let state = {
        let mut q = queue.lock().await;
        if let Some(item) = q.items.iter_mut().find(|i| i.id == item_id) {
            item.title = info.title.clone();
            item.total_bytes = info.file_size_bytes;
            let fc = if info.media_type == crate::models::media::MediaType::Carousel
                || info.media_type == crate::models::media::MediaType::Playlist
            {
                info.available_qualities.len() as u32
            } else {
                1
            };
            item.file_count = Some(fc);
            item.media_info = Some(info.clone());
        }
        q.get_state()
    };
    emit_queue_state_from_state(&app, state);

    let _ = app.emit(
        "queue-item-progress",
        &QueueItemProgress {
            id: item_id,
            title: info.title.clone(),
            platform: platform_name.clone(),
            percent: 0.5,
            speed_bytes_per_sec: 0.0,
            downloaded_bytes: 0,
            total_bytes: info.file_size_bytes,
            phase: "starting".to_string(),
            eta_seconds: None,
        },
    );

    let settings = config::load_settings(&app);
    let tmpl = settings.download.filename_template.clone();
    let mut final_output_dir = std::path::PathBuf::from(&output_dir);
    if settings.download.organize_by_platform {
        final_output_dir = final_output_dir.join(&platform_name);
    }
    let torrent_id_slot = Arc::new(tokio::sync::Mutex::new(None));
    let audio_format = if download_mode.as_deref() == Some("audio") {
        Some(settings.download.music_audio_format.clone())
    } else {
        None
    };
    let opts = crate::models::media::DownloadOptions {
        quality: quality.or_else(|| Some(settings.download.video_quality.clone())),
        output_dir: final_output_dir,
        filename_template: Some(tmpl),
        download_subtitles: settings.download.download_subtitles,
        include_auto_subtitles: settings.download.include_auto_subtitles,
        download_mode,
        audio_format,
        format_id,
        referer,
        extra_headers,
        page_url,
        user_agent,
        cancel_token: cancel_token.clone(),
        concurrent_fragments: settings.advanced.concurrent_fragments,
        ytdlp_path,
        torrent_listen_port: Some(settings.advanced.torrent_listen_port),
        torrent_id_slot: Some(torrent_id_slot.clone()),
        custom_ytdlp_args: custom_ytdlp_args.clone(),
        torrent_files: torrent_files.clone(),
        torrent_auto_trackers: settings.advanced.torrent_auto_trackers,
        torrent_upnp: settings.advanced.torrent_upnp,
    };

    let total_bytes = info.file_size_bytes;
    let item_title = info.title.clone();
    let log_title = item_title.clone();
    let item_platform = platform_name.clone();
    let (tx, mut rx) = mpsc::channel::<omniget_core::models::progress::ProgressUpdate>(32);

    let app_progress = app.clone();
    let queue_progress = queue.clone();
    let torrent_id_slot_progress = torrent_id_slot.clone();
    let progress_forwarder = tokio::spawn(async move {
        const STALL_AFTER: std::time::Duration = std::time::Duration::from_secs(6);

        let mut last_bytes: u64 = 0;
        let mut last_time = std::time::Instant::now();
        let mut throttle = ProgressThrottle::new(250);
        let mut current_speed: f64 = 0.0;
        let mut last_percent: f64 = 0.0;
        let mut last_advance = std::time::Instant::now();
        let mut stalled = false;

        loop {
            let update = tokio::select! {
                msg = rx.recv() => match msg {
                    Some(u) => u,
                    None => break,
                },
                _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
                    if !stalled && last_advance.elapsed() >= STALL_AFTER {
                        stalled = true;
                        current_speed = 0.0;
                        {
                            let mut q = queue_progress.lock().await;
                            let tid = { *torrent_id_slot_progress.lock().await };
                            q.update_progress(
                                item_id, last_percent, 0.0, last_bytes, total_bytes, tid, None,
                            );
                        }
                        let _ = app_progress.emit(
                            "queue-item-progress",
                            &QueueItemProgress {
                                id: item_id,
                                title: item_title.clone(),
                                platform: item_platform.clone(),
                                percent: last_percent,
                                speed_bytes_per_sec: 0.0,
                                downloaded_bytes: last_bytes,
                                total_bytes,
                                phase: "stalled".to_string(),
                                eta_seconds: None,
                            },
                        );
                    }
                    continue;
                }
            };

            let percent = update.percent;
            if !throttle.should_emit() && percent < 100.0 && !update.has_real_metrics() {
                continue;
            }

            let now = std::time::Instant::now();
            let resolved_total = update.total_bytes.or(total_bytes);
            let mut clamped = percent.clamp(0.0, 100.0);
            if percent >= 0.0 && percent < 100.0 {
                if clamped < last_percent {
                    clamped = last_percent;
                }

                let metric_percent = update.downloaded_bytes.and_then(|downloaded| {
                    resolved_total
                        .filter(|total| *total > 0)
                        .map(|total| (downloaded as f64 / total as f64 * 100.0).clamp(0.0, 100.0))
                });

                if let Some(metric) = metric_percent {
                    let metric_ceiling = (metric + 15.0).max(last_percent);
                    if clamped > metric_ceiling {
                        clamped = metric_ceiling;
                    }
                } else {
                    let max_step = if update.has_real_metrics() { 12.0 } else { 6.0 };
                    let ceiling = (last_percent + max_step).min(99.0);
                    if clamped > ceiling {
                        clamped = ceiling;
                    }
                }
            }

            let mut downloaded_bytes = update.downloaded_bytes.unwrap_or_else(|| {
                resolved_total
                    .map(|total| (clamped / 100.0 * total as f64) as u64)
                    .unwrap_or(last_bytes)
            });
            if downloaded_bytes < last_bytes && percent < 100.0 {
                downloaded_bytes = last_bytes;
            }

            if let Some(real) = update.speed_bps {
                current_speed = real;
            } else if downloaded_bytes > last_bytes {
                let dt = now.duration_since(last_time).as_secs_f64();
                if dt > 0.1 {
                    let instant_speed = (downloaded_bytes - last_bytes) as f64 / dt;
                    current_speed = if current_speed > 0.0 {
                        current_speed * 0.7 + instant_speed * 0.3
                    } else {
                        instant_speed
                    };
                }
            }

            if downloaded_bytes > last_bytes || clamped > last_percent || update.speed_bps.is_some()
            {
                last_advance = now;
                stalled = false;
            }
            last_bytes = downloaded_bytes;
            last_time = now;
            last_percent = clamped;

            let phase_value = if percent < 0.0 { percent } else { clamped };
            let phase = match phase_value {
                p if p < -1.5 => "connecting",
                p if p < -0.5 => "starting",
                p if p > 99.5 => "finalizing",
                p if p > 0.0 => "downloading",
                _ => "starting",
            };

            let eta_seconds = update
                .eta_seconds
                .or_else(|| omniget_core::core::ytdlp::get_eta(item_id))
                .or_else(|| {
                    if current_speed > 0.0 {
                        resolved_total.and_then(|total| {
                            (total > downloaded_bytes)
                                .then(|| ((total - downloaded_bytes) as f64 / current_speed) as u64)
                        })
                    } else {
                        None
                    }
                });

            {
                let mut q = queue_progress.lock().await;
                let tid = { *torrent_id_slot_progress.lock().await };
                q.update_progress(
                    item_id,
                    clamped,
                    current_speed,
                    downloaded_bytes,
                    resolved_total,
                    tid,
                    eta_seconds,
                );
            }

            let _ = app_progress.emit(
                "queue-item-progress",
                &QueueItemProgress {
                    id: item_id,
                    title: item_title.clone(),
                    platform: item_platform.clone(),
                    percent: clamped,
                    speed_bytes_per_sec: current_speed,
                    downloaded_bytes,
                    total_bytes: resolved_total,
                    phase: phase.to_string(),
                    eta_seconds,
                },
            );
        }
        omniget_core::core::ytdlp::clear_eta(item_id);
    });

    if let Some(ua) = opts.user_agent.clone() {
        omniget_core::core::ytdlp::register_ext_user_agent(url.clone(), ua);
    }
    if let Some(hdrs) = opts.extra_headers.clone() {
        omniget_core::core::ytdlp::register_ext_headers(url.clone(), hdrs);
    }

    let dl_start = std::time::Instant::now();
    append_download_log(
        &app,
        item_id,
        format!(
            "[omniget] starting download: platform={} title=\"{}\" url={}",
            platform_name, log_title, url
        ),
    );
    let dl_future = async {
        tokio::select! {
            r = downloader.download(&info, &opts, tx) => r,
            _ = cancel_token.cancelled() => {
                Err(anyhow::anyhow!("Download cancelado"))
            }
        }
    };
    let result = omniget_core::core::log_hook::CURRENT_COOKIE_SLUG
        .scope(
            cookie_slug.clone(),
            omniget_core::core::log_hook::CURRENT_DOWNLOAD_ID.scope(item_id, dl_future),
        )
        .await;
    omniget_core::core::ytdlp::clear_ext_user_agent(&url);
    omniget_core::core::ytdlp::clear_ext_headers(&url);
    tracing::info!(
        "[queue] download {} completed in {:?}",
        item_id,
        dl_start.elapsed()
    );

    let _ = progress_forwarder.await;

    let was_paused = {
        let q = queue.lock().await;
        q.items
            .iter()
            .find(|i| i.id == item_id)
            .map(|i| i.status == QueueStatus::Paused)
            .unwrap_or(false)
    };

    if was_paused {
        let state = {
            let q = queue.lock().await;
            q.get_state()
        };
        emit_queue_state_from_state(&app, state);
        try_start_next(app, queue).await;
        return;
    }

    match result {
        Ok(dl) => {
            append_download_log(
                &app,
                item_id,
                format!(
                    "[omniget] download finished: path={} size={} bytes",
                    dl.file_path.to_string_lossy(),
                    dl.file_size_bytes
                ),
            );
            if settings.download.embed_metadata
                && platform_name != "magnet"
                && ffmpeg::is_ffmpeg_available().await
            {
                let metadata = MetadataEmbed {
                    title: Some(info.title.clone()),
                    artist: Some(info.author.clone()),
                    thumbnail_url: info.thumbnail_url.clone(),
                    ..Default::default()
                };
                if let Err(e) = ffmpeg::embed_metadata(
                    &dl.file_path,
                    &metadata,
                    settings.download.embed_thumbnail,
                    shared_http_client(),
                )
                .await
                {
                    tracing::warn!("Metadata embed failed for '{}': {}", info.title, e);
                }
            }

            if from_hotkey && settings.download.copy_to_clipboard_on_hotkey {
                #[cfg(not(target_os = "android"))]
                {
                    match crate::core::clipboard::copy_file_to_clipboard(&dl.file_path).await {
                        Ok(()) => {
                            let _ = app.emit(
                                "file-copied-to-clipboard",
                                serde_json::json!({
                                    "path": dl.file_path.to_string_lossy(),
                                }),
                            );
                        }
                        Err(e) => {
                            tracing::warn!("[clipboard] failed to copy file: {}", e);
                        }
                    }
                }
            }

            let state = {
                let mut q = queue.lock().await;
                if platform_name == "magnet" && dl.torrent_id.is_some() {
                    q.mark_seeding(
                        item_id,
                        Some(dl.file_path.to_string_lossy().to_string()),
                        Some(dl.file_size_bytes),
                        dl.torrent_id,
                    );
                } else {
                    q.mark_complete(
                        item_id,
                        true,
                        None,
                        Some(dl.file_path.to_string_lossy().to_string()),
                        Some(dl.file_size_bytes),
                    );
                }
                q.get_state()
            };
            emit_queue_state_from_state(&app, state);
        }
        Err(e) => {
            let raw_err = e.to_string();
            append_download_log(
                &app,
                item_id,
                format!("[omniget] download failed: {}", raw_err),
            );
            let (category, hint) = omniget_core::core::errors::classify_download_error(&raw_err);
            let user_msg = if category != "unknown" {
                format!("{} ({})", hint, raw_err)
            } else {
                raw_err.clone()
            };
            tracing::error!(
                "Download error '{}' [{}]: {}",
                platform_name,
                category,
                raw_err
            );

            let retry_decision = {
                let mut q = queue.lock().await;
                if let Some(item) = q.items.iter_mut().find(|i| i.id == item_id) {
                    if item.downloaded_bytes > 5 * 1024 * 1024 {
                        item.retry_count = 0;
                    }
                    let retryable = is_retryable_category(category);
                    let attempt = item.retry_count;
                    let max = item.max_retries;
                    if retryable && attempt < max {
                        item.retry_count = attempt + 1;
                        Some((attempt + 1, max))
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            if let Some((next_attempt, max)) = retry_decision {
                let delay_secs = (1u64 << (next_attempt - 1).min(5)).min(30);
                tracing::warn!(
                    "[queue] retry {}/{} for {} in {}s (category={})",
                    next_attempt,
                    max,
                    item_id,
                    delay_secs,
                    category
                );
                let state = {
                    let mut q = queue.lock().await;
                    if let Some(item) = q.items.iter_mut().find(|i| i.id == item_id) {
                        item.status = QueueStatus::Queued;
                        item.cancel_token = CancellationToken::new();
                        item.percent = 0.0;
                        item.speed_bytes_per_sec = 0.0;
                        item.downloaded_bytes = 0;
                    }
                    q.get_state()
                };
                emit_queue_state_from_state(&app, state);
                let app_for_retry = app.clone();
                let queue_for_retry = queue.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
                    try_start_next(app_for_retry, queue_for_retry).await;
                });
                return;
            }

            let state = {
                let mut q = queue.lock().await;
                q.mark_complete(item_id, false, Some(user_msg), None, None);
                q.get_state()
            };
            emit_queue_state_from_state(&app, state);
        }
    }

    try_start_next(app, queue).await;
}

fn is_retryable_category(category: &str) -> bool {
    matches!(category, "unknown" | "rate_limited")
}

async fn fetch_and_cache_info(
    url: &str,
    downloader: &dyn PlatformDownloader,
    platform: &str,
    ytdlp_path: Option<&std::path::Path>,
) -> anyhow::Result<MediaInfo> {
    {
        let cache = info_cache().lock().await;
        if let Some(entry) = cache.get(url) {
            if entry.cached_at.elapsed() < INFO_CACHE_TTL {
                tracing::debug!("[perf] fetch_and_cache_info: cache hit for {}", platform);
                return Ok(entry.info.clone());
            }
        }
    }

    let url_lock = {
        let mut map = in_flight_map().lock().await;
        map.entry(url.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    let _guard = url_lock.lock().await;

    {
        let cache = info_cache().lock().await;
        if let Some(entry) = cache.get(url) {
            if entry.cached_at.elapsed() < INFO_CACHE_TTL {
                tracing::debug!(
                    "[perf] fetch_and_cache_info: dedup cache hit for {}",
                    platform
                );
                return Ok(entry.info.clone());
            }
        }
    }

    tracing::debug!("[perf] fetch_and_cache_info: fetching for {}", platform);
    let info = if let Some(ytdlp) = ytdlp_path {
        match platform {
            "youtube" => {
                crate::platforms::youtube::YouTubeDownloader::fetch_with_ytdlp(url, ytdlp).await?
            }
            "generic" => {
                let json = crate::core::ytdlp::get_video_info(ytdlp, url, &[]).await?;
                crate::platforms::generic_ytdlp::GenericYtdlpDownloader::parse_video_info(&json)?
            }
            _ => downloader.get_media_info(url).await?,
        }
    } else {
        downloader.get_media_info(url).await?
    };

    let mut cache = info_cache().lock().await;
    cache.insert(
        url.to_string(),
        CachedInfo {
            info: info.clone(),
            cached_at: std::time::Instant::now(),
        },
    );
    if cache.len() > 50 {
        cache.retain(|_, v| v.cached_at.elapsed() < INFO_CACHE_TTL);
    }
    Ok(info)
}

pub async fn try_get_cached_info(url: &str) -> Option<MediaInfo> {
    let cache = info_cache().lock().await;
    cache
        .get(url)
        .filter(|entry| entry.cached_at.elapsed() < INFO_CACHE_TTL)
        .map(|entry| entry.info.clone())
}

pub async fn prefetch_info(
    url: &str,
    downloader: &dyn PlatformDownloader,
    platform: &str,
    ytdlp_path: Option<&std::path::Path>,
) {
    prefetch_info_with_emit(url, downloader, platform, ytdlp_path, None).await;
}

pub async fn prefetch_info_with_emit(
    url: &str,
    downloader: &dyn PlatformDownloader,
    platform: &str,
    ytdlp_path: Option<&std::path::Path>,
    app: Option<tauri::AppHandle>,
) {
    let _timer_start = std::time::Instant::now();
    tracing::debug!("[perf] prefetch_info: started");
    match fetch_and_cache_info(url, downloader, platform, ytdlp_path).await {
        Ok(info) => {
            tracing::debug!(
                "[perf] prefetch_info: completed in {:?} — {}",
                _timer_start.elapsed(),
                info.title
            );
            if let Some(app) = app {
                let preview = MediaPreviewEvent {
                    url: url.to_string(),
                    title: info.title.clone(),
                    author: info.author.clone(),
                    thumbnail_url: info.thumbnail_url.clone(),
                    duration_seconds: info.duration_seconds,
                };
                let _ = app.emit("media-info-preview", preview);
            }
        }
        Err(e) => tracing::warn!(
            "[perf] prefetch_info: failed in {:?} — {}",
            _timer_start.elapsed(),
            e
        ),
    }
}

pub async fn try_start_next(app: tauri::AppHandle, queue: Arc<tokio::sync::Mutex<DownloadQueue>>) {
    let _timer_start = std::time::Instant::now();
    let (next_ids, stagger, state_to_emit) = {
        let mut q = queue.lock().await;
        let ids = q.next_queued_ids();
        for nid in &ids {
            q.mark_active(*nid);
        }
        let state = if !ids.is_empty() {
            Some(q.get_state())
        } else {
            None
        };
        (ids, q.stagger_delay_ms, state)
    };

    if let Some(state) = state_to_emit {
        emit_queue_state_from_state(&app, state);
    }

    let batch_size = next_ids.len();
    for (i, nid) in next_ids.into_iter().enumerate() {
        let _ = app.emit(
            "queue-item-progress",
            &QueueItemProgress {
                id: nid,
                title: String::new(),
                platform: String::new(),
                percent: 0.0,
                speed_bytes_per_sec: 0.0,
                downloaded_bytes: 0,
                total_bytes: None,
                phase: "queued_starting".to_string(),
                eta_seconds: None,
            },
        );

        if i > 0 {
            let item_platform = {
                let q = queue.lock().await;
                q.items
                    .iter()
                    .find(|item| item.id == nid)
                    .map(|item| item.platform.clone())
            };
            let delay_ms = if item_platform.as_deref() == Some("youtube") {
                2000
            } else if batch_size > 3 {
                stagger.max(1000)
            } else {
                stagger
            };
            if delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
        }
        let app_c = app.clone();
        let queue_c = queue.clone();
        tokio::spawn(async move {
            spawn_download(app_c, queue_c, nid).await;
        });
    }
    tracing::debug!("[perf] try_start_next took {:?}", _timer_start.elapsed());
}

// Periodic tick so a future-scheduled download still starts when its time
// arrives even if the queue is otherwise idle, and so a download with a
// stop time is cancelled when that time passes.
pub fn start_scheduler(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        loop {
            let state = app.state::<crate::AppState>();
            let queue = state.download_queue.clone();
            let (has_due, stopped_any) = {
                let q = queue.lock().await;
                let now = now_ms();
                let mut stopped = false;
                for item in &q.items {
                    if item.status == QueueStatus::Active {
                        if let Some(stop) = item.stop_at_ms {
                            if now >= stop {
                                item.cancel_token.cancel();
                                stopped = true;
                            }
                        }
                    }
                }
                let due = q.items.iter().any(|i| {
                    i.status == QueueStatus::Queued
                        && i.scheduled_at_ms.map(|t| now >= t).unwrap_or(false)
                });
                (due, stopped)
            };
            if has_due || stopped_any {
                try_start_next(app.clone(), queue.clone()).await;
            }
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        }
    });
}

fn is_generic_title(title: &str) -> bool {
    let t = title.to_lowercase();
    let t = t.trim();
    t.is_empty()
        || t == "video"
        || t == "media"
        || t == "untitled"
        || t == "unknown"
        || t.starts_with("video [video]")
        || t.starts_with("media [media]")
}

#[cfg(test)]
mod kind_tests {
    use super::{kind_from_platform, QueueKind};

    #[test]
    fn youtube_and_video_platforms_map_to_video() {
        assert_eq!(kind_from_platform("youtube"), QueueKind::Video);
        assert_eq!(kind_from_platform("vimeo"), QueueKind::Video);
        assert_eq!(kind_from_platform("twitch"), QueueKind::Video);
        assert_eq!(kind_from_platform("bilibili"), QueueKind::Video);
        assert_eq!(kind_from_platform("tiktok"), QueueKind::Video);
        assert_eq!(kind_from_platform("instagram"), QueueKind::Video);
        assert_eq!(kind_from_platform("reddit"), QueueKind::Video);
        assert_eq!(kind_from_platform("bluesky"), QueueKind::Video);
        assert_eq!(kind_from_platform("generic_ytdlp"), QueueKind::Video);
    }

    #[test]
    fn audio_platforms() {
        assert_eq!(kind_from_platform("soundcloud"), QueueKind::Audio);
        assert_eq!(kind_from_platform("spotify"), QueueKind::Audio);
    }

    #[test]
    fn pinterest_is_image() {
        assert_eq!(kind_from_platform("pinterest"), QueueKind::Image);
    }

    #[test]
    fn pdf_kind() {
        assert_eq!(kind_from_platform("pdf"), QueueKind::Pdf);
    }

    #[test]
    fn book_platforms() {
        assert_eq!(kind_from_platform("annas_archive"), QueueKind::Book);
        assert_eq!(kind_from_platform("libgen"), QueueKind::Book);
        assert_eq!(kind_from_platform("gutendex"), QueueKind::Book);
        assert_eq!(kind_from_platform("book"), QueueKind::Book);
    }

    #[test]
    fn webpage_kind() {
        assert_eq!(kind_from_platform("webpage"), QueueKind::Webpage);
        assert_eq!(kind_from_platform("embed"), QueueKind::Webpage);
    }

    #[test]
    fn telegram_kind() {
        assert_eq!(kind_from_platform("telegram"), QueueKind::TelegramMedia);
        assert_eq!(
            kind_from_platform("telegram_media"),
            QueueKind::TelegramMedia
        );
    }

    #[test]
    fn course_lesson_kind() {
        assert_eq!(kind_from_platform("courses"), QueueKind::CourseLesson);
        assert_eq!(kind_from_platform("course_lesson"), QueueKind::CourseLesson);
    }

    #[test]
    fn generic_for_torrents_and_p2p() {
        assert_eq!(kind_from_platform("magnet"), QueueKind::Generic);
        assert_eq!(kind_from_platform("p2p"), QueueKind::Generic);
        assert_eq!(kind_from_platform("torrent"), QueueKind::Generic);
    }

    #[test]
    fn unknown_platform_falls_back_to_generic() {
        assert_eq!(kind_from_platform(""), QueueKind::Generic);
        assert_eq!(kind_from_platform("totally-unknown"), QueueKind::Generic);
        assert_eq!(kind_from_platform("xyz123"), QueueKind::Generic);
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(kind_from_platform("YouTube"), QueueKind::Video);
        assert_eq!(kind_from_platform("TELEGRAM"), QueueKind::TelegramMedia);
    }
}
