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
use omniget_core::core::ytdlp::CommandRecord;
use omniget_core::models::progress::StreamInfo;

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
    Webpage,
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

fn settle_terminal_run_state(item: &mut QueueItem, phase: &str) {
    item.phase = Some(phase.to_string());
    item.eta_seconds = None;
    item.speed_bytes_per_sec = 0.0;
}

pub fn is_retryable_error_message(message: &str) -> bool {
    let lower = message.to_lowercase();
    if lower.contains("cancel") {
        return false;
    }
    if let Some(d) = crate::core::root_cause::diagnose_code(message) {
        return d.is_retryable();
    }
    let (category, _) = omniget_core::core::errors::classify_download_error(message);
    matches!(
        category,
        "unknown" | "rate_limited" | "server_error" | "blocked_by_platform"
    )
}

fn inspect_failure_message(platform: &str, raw: &str) -> String {
    let raw = super::flight_recorder::redact(raw);
    if platform == "mcp_worker" {
        return raw;
    }
    let (category, hint) = omniget_core::core::errors::classify_download_error(&raw);
    if category == "unknown" {
        raw.clone()
    } else {
        format!("{} ({})", hint, raw)
    }
}

fn resolve_percent(
    update: &omniget_core::models::progress::ProgressUpdate,
    resolved_total: Option<u64>,
) -> Option<f64> {
    update.percent_value().or_else(|| {
        let downloaded = update.downloaded_bytes?;
        let total = resolved_total.filter(|t| *t > 0)?;
        Some((downloaded as f64 / total as f64 * 100.0).clamp(0.0, 99.9))
    })
}

pub const LINK_EXPIRED_MESSAGE: &str =
    "LINK_EXPIRED: This link has expired or had its access key removed for privacy. Paste the original link again.";

pub fn external_retryable(message: &str) -> bool {
    !message.to_ascii_lowercase().contains("cancel")
        && crate::core::root_cause::machine_diagnose(message).is_retryable()
}

fn history_retryable(message: &str, url: &str) -> bool {
    is_retryable_error_message(message) && !crate::core::flight_recorder::is_redacted_url(url)
}

fn expire_redacted_link(item: &mut QueueItem) -> bool {
    if !crate::core::flight_recorder::is_redacted_url(&item.url) {
        return false;
    }
    item.status = QueueStatus::Error {
        message: LINK_EXPIRED_MESSAGE.to_string(),
        retryable: false,
    };
    item.phase = Some("error".into());
    true
}

#[derive(Clone, Serialize)]
pub struct QueueItemInfo {
    pub id: u64,
    pub url: String,
    pub platform: String,
    pub title: String,
    pub status: QueueStatus,
    pub percent: Option<f64>,
    pub speed_bytes_per_sec: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub file_path: Option<String>,
    pub file_size_bytes: Option<u64>,
    pub file_count: Option<u32>,
    pub thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<QueueKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<StreamInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub streams_done: Vec<StreamInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_formats: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fragment_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fragment_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<CommandRecord>,
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
    pub percent: Option<f64>,
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
    pub phase: Option<String>,
    pub current_stream: Option<StreamInfo>,
    pub streams_done: Vec<StreamInfo>,
    pub planned_formats: Option<Vec<String>>,
    pub fragment_index: Option<u32>,
    pub fragment_count: Option<u32>,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
    pub ytdlp_argv_override: Option<Vec<String>>,
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
            eta_seconds: self.eta_seconds,
            quality: self.quality.clone(),
            download_mode: self.download_mode.clone(),
            author: self
                .media_info
                .as_ref()
                .map(|m| m.author.clone())
                .filter(|a| !a.is_empty() && a != "unknown"),
            duration_seconds: self.media_info.as_ref().and_then(|m| m.duration_seconds),
            phase: self.phase.clone(),
            stream: self.current_stream.clone(),
            streams_done: self.streams_done.clone(),
            planned_formats: self.planned_formats.clone(),
            fragment_index: self.fragment_index,
            fragment_count: self.fragment_count,
            started_at_ms: self.started_at_ms,
            finished_at_ms: self.finished_at_ms,
            command: omniget_core::core::ytdlp::get_command(self.id),
        }
    }

    fn reset_run_state(&mut self) {
        self.phase = None;
        self.current_stream = None;
        self.streams_done.clear();
        self.planned_formats = None;
        self.fragment_index = None;
        self.fragment_count = None;
        self.started_at_ms = None;
        self.finished_at_ms = None;
    }
}

pub struct DownloadQueue {
    pub items: Vec<QueueItem>,
    pub max_concurrent: u32,
    pub stagger_delay_ms: u64,
    pub default_max_retries: u32,
    last_issued_id: u64,
}

fn status_after_resume(item: &QueueItem) -> QueueStatus {
    if item.platform == "magnet" && item.finished_at_ms.is_some() {
        QueueStatus::Seeding
    } else {
        QueueStatus::Active
    }
}

fn can_finish_active_item(status: &QueueStatus) -> bool {
    *status == QueueStatus::Active
}

fn run_was_cancelled(status: &QueueStatus, cancel_token: &CancellationToken) -> bool {
    can_finish_active_item(status) && cancel_token.is_cancelled()
}

impl DownloadQueue {
    pub fn new(max_concurrent: u32) -> Self {
        Self {
            items: Vec::new(),
            max_concurrent,
            stagger_delay_ms: 150,
            default_max_retries: 3,
            last_issued_id: 0,
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
            title: crate::core::flight_recorder::redact_urls(&title),
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
            percent: Some(0.0),
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
            phase: None,
            current_stream: None,
            streams_done: Vec::new(),
            planned_formats: None,
            fragment_index: None,
            fragment_count: None,
            started_at_ms: None,
            finished_at_ms: None,
            ytdlp_argv_override: None,
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

    pub fn push_expired_link(&mut self, r: &crate::core::recovery::RecoveryItem) {
        if !self.items.iter().any(|i| i.id == r.id) {
            self.enqueue(
                r.id,
                r.url.clone(),
                r.platform.clone(),
                r.title.clone(),
                r.output_dir.clone(),
                r.download_mode.clone(),
                r.quality.clone(),
                r.format_id.clone(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Arc::new(crate::platforms::noop::NoopDownloader::new()),
                None,
                false,
                None,
                None,
                None,
                None,
                None,
            );
            if let Some(item) = self.items.iter_mut().find(|i| i.id == r.id) {
                expire_redacted_link(item);
            }
        }
        crate::core::recovery::remove(r.id);
    }

    pub fn hydrate_from_history(&mut self) {
        let entries = crate::core::queue_history::list();
        if entries.is_empty() {
            return;
        }
        let placeholder: Arc<dyn PlatformDownloader> =
            Arc::new(crate::platforms::noop::NoopDownloader::new());
        for entry in entries.iter().rev() {
            self.push_history_item(entry, None, &placeholder);
        }
    }

    pub fn hydrate_entry(
        &mut self,
        entry: &crate::core::queue_history::HistoryEntry,
        retryable: bool,
    ) -> bool {
        let placeholder: Arc<dyn PlatformDownloader> =
            Arc::new(crate::platforms::noop::NoopDownloader::new());
        self.push_history_item(entry, Some(retryable), &placeholder)
    }

    fn push_history_item(
        &mut self,
        entry: &crate::core::queue_history::HistoryEntry,
        retryable: Option<bool>,
        placeholder: &Arc<dyn PlatformDownloader>,
    ) -> bool {
        if self.items.iter().any(|i| i.id == entry.id) {
            return false;
        }
        let status = if entry.success {
            QueueStatus::Complete { success: true }
        } else {
            let msg = entry.error.clone().unwrap_or_default();
            let retryable = retryable.unwrap_or_else(|| history_retryable(&msg, &entry.url));
            QueueStatus::Error {
                message: msg,
                retryable,
            }
        };
        let percent = Some(if entry.success { 100.0 } else { 0.0 });
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
            phase: Some(if entry.success { "completed" } else { "error" }.into()),
            current_stream: None,
            streams_done: Vec::new(),
            planned_formats: None,
            fragment_index: None,
            fragment_count: None,
            started_at_ms: None,
            finished_at_ms: None,
            ytdlp_argv_override: None,
        };
        self.items.push(item);
        true
    }

    pub fn active_count(&self) -> u32 {
        self.items
            .iter()
            .filter(|i| i.status == QueueStatus::Active && i.platform != "telegram")
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

    pub fn next_available_id(&mut self, preferred: u64) -> u64 {
        let mut id = preferred.max(self.last_issued_id.saturating_add(1));
        while self.items.iter().any(|i| i.id == id) {
            id = id.saturating_add(1);
        }
        self.last_issued_id = id;
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
            item.reset_run_state();
            item.started_at_ms = Some(now_ms());
            super::download_journal::begin_attempt(id);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_run_state(
        &mut self,
        id: u64,
        phase: Option<&str>,
        stream: Option<&StreamInfo>,
        fragment: Option<(u32, u32)>,
        planned: Option<&Vec<String>>,
    ) {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            if item.status != QueueStatus::Active {
                return;
            }
            if let Some(p) = phase {
                item.phase = Some(p.to_string());
            }
            if let Some(s) = stream {
                let changed = item
                    .current_stream
                    .as_ref()
                    .map(|c| c.format_id != s.format_id)
                    .unwrap_or(true);
                if changed {
                    if let Some(prev) = item.current_stream.take() {
                        if !item
                            .streams_done
                            .iter()
                            .any(|d| d.format_id == prev.format_id)
                        {
                            item.streams_done.push(prev);
                        }
                    }
                    item.current_stream = Some(s.clone());
                }
            }
            if let Some((i, c)) = fragment {
                item.fragment_index = Some(i);
                item.fragment_count = Some(c);
            }
            if let Some(p) = planned {
                item.planned_formats = Some(p.clone());
            }
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
            if !can_finish_active_item(&item.status) {
                return;
            }
            let error = error.map(|e| super::flight_recorder::redact(&e));
            let (success, error) = if item.platform == "mcp_worker" {
                let retryable = error.as_deref().is_some_and(external_retryable);
                match crate::mcp::download_intents::terminal_receipt(
                    id,
                    success,
                    error.clone(),
                    file_path.clone(),
                    file_size_bytes,
                    retryable,
                ) {
                    Ok(()) => (success, error),
                    Err(e) => {
                        tracing::warn!("[mcp] terminal receipt unavailable for {}: {}", id, e);
                        let _ = crate::mcp::download_intents::unknown(id);
                        (false,Some("OUTCOME_UNKNOWN: terminal receipt unavailable; output evidence retained".into()))
                    }
                }
            } else {
                (success, error)
            };
            super::download_journal::record(
                id,
                &format!(
                    "[finalization] {}: {}",
                    if success { "completed" } else { "error" },
                    error.as_deref().unwrap_or("")
                ),
            );
            let error_for_history = error.clone();
            if success {
                item.status = QueueStatus::Complete { success: true };
                item.percent = Some(100.0);
                item.phase = Some("completed".into());
            } else {
                let msg = error.unwrap_or_default();
                let retryable = if item.platform == "mcp_worker" {
                    external_retryable(&msg)
                        && crate::mcp::download_intents::receipt(id)
                            .ok()
                            .flatten()
                            .is_some_and(|r| r.retryable)
                } else {
                    is_retryable_error_message(&msg)
                };
                item.status = QueueStatus::Error {
                    message: msg,
                    retryable,
                };
                item.phase = Some("error".into());
            }
            item.file_path = file_path;
            item.file_size_bytes = file_size_bytes;
            item.speed_bytes_per_sec = 0.0;
            item.eta_seconds = None;
            item.finished_at_ms = Some(now_ms());
            crate::core::recovery::remove(id);

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

    pub fn mark_cancelled(&mut self, id: u64) {
        let cancelled = self
            .items
            .iter()
            .any(|i| i.id == id && run_was_cancelled(&i.status, &i.cancel_token));
        if cancelled {
            self.mark_complete(id, false, Some("Download cancelled".into()), None, None);
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
            if !can_finish_active_item(&item.status) {
                return;
            }
            item.status = QueueStatus::Seeding;
            item.percent = Some(100.0);
            item.file_path = file_path;
            item.file_size_bytes = file_size_bytes;
            item.speed_bytes_per_sec = 0.0;
            item.torrent_id = torrent_id;
            item.finished_at_ms = Some(now_ms());
            crate::core::recovery::remove(id);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_progress(
        &mut self,
        id: u64,
        percent: Option<f64>,
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
            if item.status == QueueStatus::Seeding {
                item.status = QueueStatus::Paused;
                item.speed_bytes_per_sec = 0.0;
                item.eta_seconds = None;
                return true;
            }
            if item.status == QueueStatus::Active {
                if item.platform == "mcp_worker" {
                    item.cancel_token.cancel();
                    item.status = QueueStatus::Paused;
                    item.phase = Some("paused".into());
                    item.speed_bytes_per_sec = 0.0;
                    item.eta_seconds = None;
                    return true;
                }
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
                if item.platform == "mcp_worker" {
                    return false;
                }
                if item.platform != "magnet"
                    && !omniget_core::core::ytdlp::resume_download_process(id)
                {
                    return false;
                }
                item.status = status_after_resume(item);
                return true;
            }
        }
        false
    }

    pub fn pause_all(&mut self) -> Vec<(u64, Option<usize>)> {
        let mut paused = Vec::new();
        for item in self.items.iter_mut() {
            if item.status == QueueStatus::Seeding {
                item.status = QueueStatus::Paused;
                item.speed_bytes_per_sec = 0.0;
                item.eta_seconds = None;
                paused.push((item.id, item.torrent_id));
            } else if item.status == QueueStatus::Active {
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
                item.status = status_after_resume(item);
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

    pub fn cancel(&mut self, id: u64) -> (bool, Option<usize>) {
        let result = self.cancel_inner(id);
        if result.0 {
            crate::core::recovery::remove(id);
            if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
                settle_terminal_run_state(item, "cancelled");
            }
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
                    if item.platform == "mcp_worker" {
                        if let Ok(Some(intent)) = crate::mcp::download_intents::load(id) {
                            if let Err(e) = crate::mcp::download_intents::settled(
                                id,
                                intent.attempt,
                                Some("cancelled"),
                            ) {
                                tracing::warn!(
                                    "[mcp] queued cancellation receipt unavailable: {}",
                                    e
                                );
                            }
                        }
                    }
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

    pub fn retry(&mut self, id: u64) -> Result<(), String> {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            if item.platform == "mcp_worker" {
                return Err("Download cannot be retried".into());
            }
            if matches!(item.status, QueueStatus::Error { .. }) {
                if expire_redacted_link(item) {
                    return Err(LINK_EXPIRED_MESSAGE.into());
                }
                item.status = QueueStatus::Queued;
                item.cancel_token = CancellationToken::new();
                item.percent = Some(0.0);
                item.speed_bytes_per_sec = 0.0;
                item.downloaded_bytes = 0;
                item.file_path = None;
                item.file_size_bytes = None;
                item.retry_count = 0;
                item.ytdlp_argv_override = None;
                item.reset_run_state();
                return Ok(());
            }
        }
        Err("Download cannot be retried".into())
    }

    pub fn retry_with_command(&mut self, id: u64, argv: Vec<String>) -> Result<(), String> {
        let item = self
            .items
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| "Download not found".to_string())?;
        if item.platform == "mcp_worker" {
            return Err("EXPLICIT_EXTERNAL_ATTEMPT_REQUIRED".into());
        }
        if !matches!(
            item.status,
            QueueStatus::Error { .. } | QueueStatus::Complete { .. }
        ) {
            return Err("Download is still running".to_string());
        }
        if crate::core::flight_recorder::is_redacted_url(&item.url) {
            if matches!(item.status, QueueStatus::Error { .. }) {
                expire_redacted_link(item);
            }
            return Err(LINK_EXPIRED_MESSAGE.into());
        }
        if argv.iter().any(|a| a.contains("[REDACTED]")) {
            return Err(LINK_EXPIRED_MESSAGE.into());
        }
        if omniget_core::core::ytdlp::get_command(id).is_none() {
            return Err(
                "This download did not run yt-dlp; there is no command to edit".to_string(),
            );
        }
        item.status = QueueStatus::Queued;
        item.cancel_token = CancellationToken::new();
        item.percent = Some(0.0);
        item.speed_bytes_per_sec = 0.0;
        item.downloaded_bytes = 0;
        item.file_path = None;
        item.file_size_bytes = None;
        item.retry_count = 0;
        item.max_retries = 0;
        item.ytdlp_argv_override = Some(argv);
        item.reset_run_state();
        Ok(())
    }

    pub fn remove(&mut self, id: u64) -> Option<Option<usize>> {
        let result = self.remove_inner(id);
        if result.is_some() {
            crate::core::recovery::remove(id);
            crate::core::queue_history::remove(id);
            omniget_core::core::ytdlp::clear_command(id);
            omniget_core::core::ytdlp::leftovers::forget_task_destinations(id);
        }
        result
    }

    fn remove_inner(&mut self, id: u64) -> Option<Option<usize>> {
        if let Some(pos) = self.items.iter().position(|i| i.id == id) {
            let item = &self.items[pos];
            if item.status == QueueStatus::Active {
                item.cancel_token.cancel();
            }
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
            .filter(|i| matches!(i.status, QueueStatus::Complete { .. }))
            .map(|i| i.id)
            .collect();
        for id in &to_remove {
            crate::core::recovery::remove(*id);
            crate::core::queue_history::remove(*id);
        }
        self.items
            .retain(|i| !matches!(i.status, QueueStatus::Complete { .. }));
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

fn serialize_display_title<S: serde::Serializer>(title: &str, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&crate::core::flight_recorder::redact_urls(title))
}

#[derive(Clone, Serialize, Default)]
pub struct QueueItemProgress {
    pub id: u64,
    #[serde(serialize_with = "serialize_display_title")]
    pub title: String,
    pub platform: String,
    pub percent: Option<f64>,
    pub speed_bytes_per_sec: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<StreamInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fragment_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fragment_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_formats: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peers: Option<u32>,
}

pub fn openable_source_url(url: &str) -> Result<String, String> {
    if crate::core::flight_recorder::is_redacted_url(url) {
        return Err(LINK_EXPIRED_MESSAGE.to_string());
    }
    Ok(url.to_string())
}

pub fn redacted_for_display(mut state: Vec<QueueItemInfo>) -> Vec<QueueItemInfo> {
    use crate::core::flight_recorder::{redact_url, redact_urls};
    for item in &mut state {
        item.url = redact_url(&item.url);
        item.title = redact_urls(&item.title);
        if let Some(t) = item.thumbnail_url.as_mut() {
            *t = redact_url(t);
        }
        if let QueueStatus::Error { message, .. } = &mut item.status {
            *message = redact_urls(message);
        }
        if let Some(cmd) = item.command.as_mut() {
            for a in &mut cmd.args {
                *a = redact_urls(a);
            }
            cmd.display = redact_urls(&cmd.display);
        }
    }
    state
}

pub fn emit_queue_state_from_state(app: &tauri::AppHandle, state: Vec<QueueItemInfo>) {
    let state = redacted_for_display(state);
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
    let known: Vec<f64> = active_items.iter().filter_map(|i| i.percent).collect();
    let avg_percent = if !known.is_empty() {
        let sum: f64 = known.iter().sum();
        sum / known.len() as f64 / 100.0
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
        let _power_hold = omniget_core::core::washmybelly::power::auto_hold_begin("download");
        let external_attempt = {
            let q = queue.lock().await;
            q.items
                .iter()
                .find(|i| i.id == item_id && i.platform == "mcp_worker")
                .and_then(|_| crate::mcp::download_intents::load(item_id).ok().flatten())
                .map(|i| i.attempt)
        };
        spawn_download_inner(app.clone(), queue.clone(), item_id).await;
        if let Some(attempt) = external_attempt {
            let q = queue.lock().await;
            let cancelled = q.items.iter().find(|i| i.id == item_id).and_then(|item| {
                if item.status == QueueStatus::Paused {
                    Some("paused")
                } else if item.cancel_token.is_cancelled() {
                    Some("cancelled")
                } else {
                    None
                }
            });
            let settled = crate::mcp::download_intents::settled(item_id, attempt, cancelled);
            if let Err(e) = &settled {
                tracing::warn!("[mcp] worker settlement not durable: {}", e);
            }
            let failed = q.items.iter().any(|i| {
                i.id == item_id
                    && matches!(
                        i.status,
                        QueueStatus::Error { .. } | QueueStatus::Complete { success: false }
                    )
            });
            drop(q);
            if settled.is_ok() && failed {
                match tokio::task::spawn_blocking(move || {
                    crate::mcp::download_intents::cleanup_failed_attempt(item_id)
                })
                .await
                {
                    Ok(Ok(removed)) if !removed.is_empty() => append_download_log(
                        &app,
                        item_id,
                        format!(
                            "[cleanup] removed {} leftover file(s) from the job folder: {}",
                            removed.len(),
                            removed.join(", ")
                        ),
                    ),
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => append_download_log(
                        &app,
                        item_id,
                        format!("[cleanup] job folder not cleaned: {e}"),
                    ),
                    Err(_) => append_download_log(
                        &app,
                        item_id,
                        "[cleanup] job folder not cleaned: CLEANUP_UNAVAILABLE",
                    ),
                }
            }
        }
        slot.disarm();
        tracing::debug!(
            "[perf] spawn_download {} took {:?}",
            item_id,
            _timer_start.elapsed()
        );
    })
}

async fn host_slot_unless_cancelled(
    host_key: &str,
    cancel_token: &CancellationToken,
) -> Option<crate::core::host_limiter::HostLease> {
    tokio::select! {
        biased;
        _ = cancel_token.cancelled() => None,
        lease = crate::core::host_limiter::acquire(host_key) => Some(lease),
    }
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
            percent: Some(0.0),
            speed_bytes_per_sec: 0.0,
            downloaded_bytes: 0,
            total_bytes: None,
            phase: "preparing".to_string(),
            eta_seconds: None,
            ..Default::default()
        },
    );

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
        argv_override,
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
            item.ytdlp_argv_override.clone(),
        )
    };

    let host_key = crate::core::host_limiter::host_key_for_url(&url);
    let Some(_host_lease) = host_slot_unless_cancelled(&host_key, &cancel_token).await else {
        if platform_name != "mcp_worker" {
            let state = {
                let mut q = queue.lock().await;
                q.mark_cancelled(item_id);
                q.get_state()
            };
            emit_queue_state_from_state(&app, state);
            try_start_next(app, queue).await;
        }
        return;
    };

    let external_intent = if platform_name == "mcp_worker" {
        loop {
            match crate::mcp::download_intents::before_execute(item_id) {
                Ok(intent) if intent.options.url == url => break Some(intent),
                Err(error) if error.starts_with("RETRY_COOLDOWN_UNTIL:") => {
                    let until = error
                        .split(':')
                        .nth(1)
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(u64::MAX);
                    let wait = until.saturating_sub(now_ms()).clamp(1, 60_000);
                    tokio::select! {
                        _=cancel_token.cancelled()=>return,
                        _=tokio::time::sleep(std::time::Duration::from_millis(wait))=>{},
                    }
                }
                result => {
                    let error = result
                        .err()
                        .unwrap_or_else(|| "DOWNLOAD_INTENT_MISMATCH".into());
                    let snapshot = {
                        let mut q = queue.lock().await;
                        q.mark_complete(item_id, false, Some(error), None, None);
                        q.get_state()
                    };
                    emit_queue_state_from_state(&app, snapshot);
                    try_start_next(app, queue).await;
                    return;
                }
            }
        }
    } else {
        None
    };

    if platform_name != "mcp_worker" {
        let settings = crate::storage::config::load_settings(&app);
        let proxy = settings.proxy.clone();
        crate::core::http_client::init_proxy(proxy.clone());
        let proxy_status = if !proxy.enabled {
            "disabled; system/env proxy honored if set".to_string()
        } else if proxy.host.trim().is_empty() {
            "enabled but host is empty; falling back to system/env proxy".to_string()
        } else {
            format!(
                "enabled; {}://{}:{}",
                proxy.proxy_type, proxy.host, proxy.port
            )
        };
        append_download_log(
            &app,
            item_id,
            format!("[network] proxy setting: {}", proxy_status),
        );
    } else {
        append_download_log(
            &app,
            item_id,
            "[network] isolated worker; mediated egress policy applies",
        );
    }

    let info_start = std::time::Instant::now();
    let info = match media_info {
        Some(i) if !i.available_qualities.is_empty() => {
            tracing::info!(
                "[queue] info for {} from cache/pre-fetched in {:?}",
                item_id,
                info_start.elapsed()
            );
            append_download_log(
                &app,
                item_id,
                format!(
                    "[omniget] using cached video info: platform={} title=\"{}\"",
                    platform_name, i.title
                ),
            );
            i
        }
        _ => {
            tracing::debug!(
                "[perf] spawn_download_inner {}: media_info is None, fetching info",
                item_id
            );
            append_download_log(
                &app,
                item_id,
                format!(
                    "[omniget] fetching video info: platform={} url={}",
                    platform_name, url
                ),
            );
            if let Some(slug) = cookie_slug.as_deref() {
                append_download_log(
                    &app,
                    item_id,
                    format!("[cookies] selected managed cookie account: {}", slug),
                );
            }
            let _ = app.emit(
                "queue-item-progress",
                &QueueItemProgress {
                    id: item_id,
                    title: crate::core::flight_recorder::redact_url(&url),
                    platform: platform_name.clone(),
                    percent: Some(0.0),
                    speed_bytes_per_sec: 0.0,
                    downloaded_bytes: 0,
                    total_bytes: None,
                    phase: "fetching_info".to_string(),
                    eta_seconds: None,
                    ..Default::default()
                },
            );

            let info_future =
                fetch_and_cache_info(&url, &*downloader, &platform_name, ytdlp_path.as_deref());
            let scoped_info_future = omniget_core::core::log_hook::CURRENT_COOKIE_SLUG.scope(
                cookie_slug.clone(),
                omniget_core::core::log_hook::CURRENT_DOWNLOAD_ID.scope(item_id, info_future),
            );
            let info_timeout_secs = if platform_name == "youtube"
                || url.to_ascii_lowercase().contains("youtube.com")
                || url.to_ascii_lowercase().contains("youtu.be")
            {
                omniget_core::core::ytdlp::YOUTUBE_VIDEO_INFO_TOTAL_TIMEOUT_SECS
            } else if platform_name == "douyin" {
                30
            } else {
                omniget_core::core::ytdlp::DEFAULT_VIDEO_INFO_TOTAL_TIMEOUT_SECS
            };
            let info_result = if platform_name == "mcp_worker" {
                let worker_future = crate::mcp::worker::INSPECT_CANCEL
                    .scope(cancel_token.clone(), scoped_info_future);
                tokio::pin!(worker_future);
                tokio::select! {
                    result=&mut worker_future=>Ok(result),
                    _=tokio::time::sleep(std::time::Duration::from_secs(info_timeout_secs))=>{
                        cancel_token.cancel();
                        let cleanup=worker_future.await;
                        if cleanup.as_ref().err().is_some_and(|e|e.to_string().contains("WORKER_TERMINATION_UNCONFIRMED")) {let _=crate::mcp::download_intents::unknown(item_id);}
                        Err(())
                    }
                }
            } else {
                tokio::select! {
                    biased;
                    _ = cancel_token.cancelled() => {
                        let state = {
                            let mut q = queue.lock().await;
                            q.mark_cancelled(item_id);
                            q.get_state()
                        };
                        emit_queue_state_from_state(&app, state);
                        try_start_next(app, queue).await;
                        return;
                    }
                    result = tokio::time::timeout(
                        std::time::Duration::from_secs(info_timeout_secs),
                        scoped_info_future,
                    ) => result.map_err(|_|()),
                }
            };

            if platform_name == "mcp_worker"
                && matches!(&info_result,Ok(Err(e)) if e.to_string().contains("WORKER_TERMINATION_UNCONFIRMED"))
            {
                let _ = crate::mcp::download_intents::unknown(item_id);
            }
            match info_result {
                Ok(Ok(i)) => {
                    append_download_log(
                        &app,
                        item_id,
                        format!(
                            "[omniget] video info fetched in {:.1}s: title=\"{}\"",
                            info_start.elapsed().as_secs_f64(),
                            i.title
                        ),
                    );
                    i
                }
                Ok(Err(e)) => {
                    append_download_log(
                        &app,
                        item_id,
                        format!(
                            "[omniget] failed fetching video info after {:.1}s: {}",
                            info_start.elapsed().as_secs_f64(),
                            e
                        ),
                    );
                    let message = inspect_failure_message(&platform_name, &e.to_string());
                    let state = {
                        let mut q = queue.lock().await;
                        q.mark_complete(item_id, false, Some(message), None, None);
                        q.get_state()
                    };
                    emit_queue_state_from_state(&app, state);
                    try_start_next(app, queue).await;
                    return;
                }
                Err(_) => {
                    tracing::warn!(
                        "[queue] info fetch timed out for {} after {}s",
                        item_id,
                        info_timeout_secs
                    );
                    append_download_log(
                        &app,
                        item_id,
                        format!(
                            "[omniget] video info timed out after {}s",
                            info_timeout_secs
                        ),
                    );
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
    if platform_name == "mcp_worker" && cancel_token.is_cancelled() {
        return;
    }
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
            item.title = crate::core::flight_recorder::redact_urls(&info.title);
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
            percent: Some(0.5),
            speed_bytes_per_sec: 0.0,
            downloaded_bytes: 0,
            total_bytes: info.file_size_bytes,
            phase: "starting".to_string(),
            eta_seconds: None,
            ..Default::default()
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
    let custom_ytdlp_args = {
        let mut args = custom_ytdlp_args.clone();
        if settings.download.skip_existing {
            let flags = args.get_or_insert_with(Vec::new);
            if !flags.iter().any(|f| f == "--no-overwrites") {
                flags.push("--no-overwrites".to_string());
            }
        }
        args
    };
    let mut opts = crate::models::media::DownloadOptions {
        quality: quality.or_else(|| Some(settings.download.video_quality.clone())),
        output_dir: final_output_dir,
        filename_template: if platform_name == "mcp_worker" {
            None
        } else {
            Some(tmpl)
        },
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
        custom_ytdlp_args: if platform_name == "mcp_worker" {
            None
        } else {
            custom_ytdlp_args.clone()
        },
        torrent_files: torrent_files.clone(),
        torrent_auto_trackers: settings.advanced.torrent_auto_trackers,
        torrent_upnp: settings.advanced.torrent_upnp,
        torrent_stop_seeding: settings.advanced.torrent_stop_seeding,
        playlist_number_files: settings.download.playlist_number_files,
    };

    if let Some(intent) = external_intent.as_ref() {
        opts.output_dir = intent.destination.clone().into();
        opts.quality = intent.options.quality.clone();
        opts.download_mode = intent.options.mode.clone();
        opts.format_id = intent.options.format_id.clone();
        opts.audio_format = intent.options.audio_format.clone();
        opts.download_subtitles = intent.options.subtitles;
        opts.include_auto_subtitles = intent.options.auto_subtitles;
        opts.concurrent_fragments = intent
            .options
            .fragments
            .clamp(1, crate::mcp::download_intents::WORKER_MAX_FRAGMENTS);
    }

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
        let mut last_known = true;
        let mut last_advance = std::time::Instant::now();
        let mut stalled = false;
        let mut current_stream: Option<StreamInfo> = None;
        let mut current_fragment: Option<(u32, u32)> = None;

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
                                item_id, last_known.then_some(last_percent), 0.0, last_bytes, total_bytes, tid, None,
                            );
                        }
                        let _ = app_progress.emit(
                            "queue-item-progress",
                            &QueueItemProgress {
                                id: item_id,
                                title: item_title.clone(),
                                platform: item_platform.clone(),
                                percent: last_known.then_some(last_percent),
                                speed_bytes_per_sec: 0.0,
                                downloaded_bytes: last_bytes,
                                total_bytes,
                                phase: "stalled".to_string(),
                                eta_seconds: None,
                                ..Default::default()
                            },
                        );
                    }
                    continue;
                }
            };

            let now = std::time::Instant::now();
            let resolved_total = update.total_bytes.or(total_bytes);
            let reported = resolve_percent(&update, resolved_total);
            let known = reported.is_some();
            let percent = reported.unwrap_or(0.0);
            if !throttle.should_emit()
                && percent < 100.0
                && !update.has_real_metrics()
                && !update.is_structural()
            {
                continue;
            }
            if let Some(s) = update.stream.as_ref() {
                let changed = current_stream
                    .as_ref()
                    .map(|c: &StreamInfo| c.format_id != s.format_id)
                    .unwrap_or(true);
                if changed {
                    current_stream = Some(s.clone());
                }
            }
            if let (Some(i), Some(c)) = (update.fragment_index, update.fragment_count) {
                current_fragment = Some((i, c));
            }

            let mut clamped = percent.clamp(0.0, 100.0);
            if (0.0..100.0).contains(&percent) {
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
                    .filter(|_| known)
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
            last_known = known;

            let phase_value = if percent < 0.0 {
                percent
            } else if !known && downloaded_bytes > 0 {
                50.0
            } else {
                clamped
            };
            let stream_phase = match current_stream.as_ref() {
                Some(s) if s.has_video() => "downloading_video",
                Some(s) if s.has_audio() => "downloading_audio",
                _ => "downloading",
            };
            let phase = if let Some(ref custom_phase) = update.phase {
                custom_phase.as_str()
            } else {
                match phase_value {
                    p if p < -1.5 => "connecting",
                    p if p < -0.5 => "starting",
                    p if p > 99.5 => "finalizing",
                    p if p > 0.0 => stream_phase,
                    _ => "starting",
                }
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
                    known.then_some(clamped),
                    current_speed,
                    downloaded_bytes,
                    resolved_total,
                    tid,
                    eta_seconds,
                );
                q.update_run_state(
                    item_id,
                    Some(phase),
                    update.stream.as_ref(),
                    current_fragment,
                    update.planned_formats.as_ref(),
                );
            }

            let _ = app_progress.emit(
                "queue-item-progress",
                &QueueItemProgress {
                    id: item_id,
                    title: item_title.clone(),
                    platform: item_platform.clone(),
                    percent: known.then_some(clamped),
                    speed_bytes_per_sec: current_speed,
                    downloaded_bytes,
                    total_bytes: resolved_total,
                    phase: phase.to_string(),
                    eta_seconds,
                    stream: current_stream.clone(),
                    fragment_index: current_fragment.map(|f| f.0),
                    fragment_count: current_fragment.map(|f| f.1),
                    planned_formats: update.planned_formats.clone(),
                    peers: update.peers,
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
        if platform_name == "mcp_worker" {
            downloader.download(&info, &opts, tx).await
        } else {
            tokio::select! {
                r = downloader.download(&info, &opts, tx) => r,
                _ = cancel_token.cancelled() => {
                    Err(anyhow::anyhow!("Download cancelled"))
                }
            }
        }
    };
    if let Some(argv) = argv_override.as_ref() {
        append_download_log(
            &app,
            item_id,
            format!(
                "[omniget] running user-edited command ({} args), single attempt",
                argv.len()
            ),
        );
    }
    let result = omniget_core::core::log_hook::CURRENT_ARGV_OVERRIDE
        .scope(
            argv_override.clone(),
            omniget_core::core::log_hook::CURRENT_COOKIE_SLUG.scope(
                cookie_slug.clone(),
                omniget_core::core::log_hook::CURRENT_DOWNLOAD_ID.scope(item_id, dl_future),
            ),
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

    if platform_name == "mcp_worker"
        && result
            .as_ref()
            .err()
            .is_some_and(|e| e.to_string().contains("WORKER_TERMINATION_UNCONFIRMED"))
    {
        let _ = crate::mcp::download_intents::unknown(item_id);
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
            let is_seeding = platform_name == "magnet" && dl.torrent_id.is_some();
            if !is_seeding {
                if let Err(msg) = validate_download_output(&dl.file_path).await {
                    tracing::error!(
                        "[queue] download {} reported success but output is missing or empty: {:?}",
                        item_id,
                        dl.file_path
                    );
                    append_download_log(
                        &app,
                        item_id,
                        format!(
                            "[omniget] download reported success but output missing or empty: {}",
                            dl.file_path.to_string_lossy()
                        ),
                    );
                    let state = {
                        let mut q = queue.lock().await;
                        q.mark_complete(item_id, false, Some(msg), None, None);
                        q.get_state()
                    };
                    emit_queue_state_from_state(&app, state);
                    try_start_next(app, queue).await;
                    return;
                }
            }

            let thumbnails_only = opts
                .custom_ytdlp_args
                .as_deref()
                .is_some_and(|args| args.iter().any(|a| a == "--skip-download"));
            if settings.download.embed_metadata
                && !thumbnails_only
                && dl.file_path.is_file()
                && platform_name != "mcp_worker"
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

            let ceiling = opts
                .format_id
                .as_deref()
                .and_then(|f| f.strip_prefix("bv*[height<="))
                .and_then(|f| f.split(']').next())
                .and_then(|h| h.parse::<u64>().ok());
            let audio_only =
                opts.download_mode.as_deref() == Some("audio") && dl.file_path.is_file();
            if ceiling.is_some() || audio_only || external_intent.is_some() {
                let probe = if let Some(intent) = external_intent.as_ref() {
                    match intent.destination_identity.as_ref() {
                        Some(identity) => {
                            super::artifact_validation::probe_authorized(
                                std::path::Path::new(&intent.destination),
                                identity,
                                &dl.file_path,
                            )
                            .await
                        }
                        None => Err("ARTIFACT_ROOT_IDENTITY_MISSING".into()),
                    }
                } else {
                    super::artifact_validation::probe(&dl.file_path).await
                };
                let validation = match probe {
                    Ok(probe) => {
                        if audio_only {
                            super::artifact_validation::check_audio(&probe)
                        } else if ceiling.is_some() {
                            super::artifact_validation::check_height(&probe, ceiling.unwrap())
                        } else {
                            Ok(())
                        }
                    }
                    Err(error) => Err(error),
                };
                if let Err(error) = validation {
                    append_download_log(&app, item_id, &error);
                    let state = {
                        let mut q = queue.lock().await;
                        q.mark_complete(item_id, false, Some(error), None, None);
                        q.get_state()
                    };
                    emit_queue_state_from_state(&app, state);
                    try_start_next(app, queue).await;
                    return;
                }
            }

            if settings.download.write_nfo_sidecar
                && platform_name != "mcp_worker"
                && !is_seeding
                && crate::core::nfo_sidecar::applies(&info)
            {
                match crate::core::nfo_sidecar::write(&dl.file_path, &info, &url).await {
                    Ok(path) => append_download_log(
                        &app,
                        item_id,
                        format!("[omniget] wrote NFO sidecar: {}", path.to_string_lossy()),
                    ),
                    Err(e) => tracing::warn!("NFO sidecar failed for '{}': {}", info.title, e),
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

            let final_size = tokio::fs::metadata(&dl.file_path)
                .await
                .map(|m| m.len())
                .unwrap_or(dl.file_size_bytes);
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
                        Some(final_size),
                    );
                }
                q.get_state()
            };
            emit_queue_state_from_state(&app, state);
        }
        Err(e) => {
            let raw_err = super::flight_recorder::redact(&e.to_string());
            append_download_log(
                &app,
                item_id,
                format!("[omniget] download failed: {}", raw_err),
            );
            let (category, hint) = omniget_core::core::errors::classify_download_error(&raw_err);
            let user_msg = if platform_name == "mcp_worker" {
                raw_err.clone()
            } else if category != "unknown"
                && !raw_err.starts_with("Post-processing failed at the ")
            {
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
                    if !can_auto_retry_item(&item.status, &item.cancel_token, category) {
                        None
                    } else {
                        let attempt = item.retry_count;
                        let max = item.max_retries;
                        if attempt < max {
                            item.retry_count = attempt + 1;
                            Some((attempt + 1, max))
                        } else {
                            None
                        }
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
                        item.percent = Some(0.0);
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
    matches!(category, "unknown" | "rate_limited" | "server_error")
}

fn can_auto_retry_item(
    status: &QueueStatus,
    cancel_token: &CancellationToken,
    category: &str,
) -> bool {
    can_finish_active_item(status)
        && !cancel_token.is_cancelled()
        && is_retryable_category(category)
}

const OUTPUT_MISSING_ERROR: &str =
    "Download reported success but the file is missing or empty. Check disk space and antivirus exclusions, then retry.";

async fn validate_download_output(path: &std::path::Path) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err(OUTPUT_MISSING_ERROR.to_string());
    }
    let meta = match tokio::fs::metadata(path).await {
        Ok(m) => m,
        Err(_) => return Err(OUTPUT_MISSING_ERROR.to_string()),
    };
    if meta.is_dir() {
        let mut entries = match tokio::fs::read_dir(path).await {
            Ok(e) => e,
            Err(_) => return Err(OUTPUT_MISSING_ERROR.to_string()),
        };
        match entries.next_entry().await {
            Ok(Some(_)) => Ok(()),
            _ => Err(OUTPUT_MISSING_ERROR.to_string()),
        }
    } else if meta.len() > 0 {
        Ok(())
    } else {
        Err(OUTPUT_MISSING_ERROR.to_string())
    }
}

async fn fetch_and_cache_info(
    url: &str,
    downloader: &dyn PlatformDownloader,
    platform: &str,
    ytdlp_path: Option<&std::path::Path>,
) -> anyhow::Result<MediaInfo> {
    if downloader.name() == "mcp_worker" {
        return downloader.get_media_info(url).await;
    }
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
                omniget_core::platforms::YouTubeDownloader::fetch_with_ytdlp(url, ytdlp).await?
            }
            "generic" => {
                if omniget_core::platforms::generic_ytdlp::is_direct_media_url(url).is_some() {
                    downloader.get_media_info(url).await?
                } else {
                    let json = crate::core::ytdlp::get_video_info(ytdlp, url, &[]).await?;
                    omniget_core::platforms::generic_ytdlp::GenericYtdlpDownloader::parse_video_info(
                        &json,
                    )?
                }
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

fn prefetch_slots() -> &'static tokio::sync::Semaphore {
    static SLOTS: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    SLOTS.get_or_init(|| tokio::sync::Semaphore::new(2))
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
    let _slot = match tokio::time::timeout(
        std::time::Duration::from_secs(20),
        prefetch_slots().acquire(),
    )
    .await
    {
        Ok(Ok(permit)) => permit,
        _ => {
            tracing::debug!("[perf] prefetch_info: skipped, too many prefetches in flight");
            return;
        }
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        fetch_and_cache_info(url, downloader, platform, ytdlp_path),
    )
    .await;
    match result {
        Ok(Ok(info)) => {
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
        Ok(Err(e)) => tracing::warn!(
            "[perf] prefetch_info: failed in {:?} — {}",
            _timer_start.elapsed(),
            e
        ),
        Err(_) => tracing::warn!(
            "[perf] prefetch_info: timed out after {:?}",
            _timer_start.elapsed()
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
                percent: Some(0.0),
                speed_bytes_per_sec: 0.0,
                downloaded_bytes: 0,
                total_bytes: None,
                phase: "queued_starting".to_string(),
                eta_seconds: None,
                ..Default::default()
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
mod source_url_tests {
    use super::*;
    use crate::core::flight_recorder::{redact, redact_url};

    const PRIVATE: &str =
        "https://site.test/watch/a1b2c3d4e5f6a7b8c9d0e1f2a3b4?video=998877&token=abc123";

    #[test]
    fn the_browser_opens_the_exact_link_while_logs_hide_its_private_parts() {
        assert_eq!(openable_source_url(PRIVATE).unwrap(), PRIVATE);
        for shown in [redact_url(PRIVATE), redact(PRIVATE)] {
            assert!(shown.contains("[REDACTED]"), "{shown}");
            assert!(!shown.contains("998877"), "{shown}");
            assert!(!shown.contains("abc123"), "{shown}");
        }
    }

    #[test]
    fn a_link_saved_redacted_is_reported_as_expired_not_opened() {
        let err = openable_source_url(&redact_url(PRIVATE)).unwrap_err();
        assert!(err.starts_with("LINK_EXPIRED"), "{err}");
    }
}

#[cfg(test)]
mod seeding_tests {
    use super::*;

    fn queue_with_torrent() -> DownloadQueue {
        let mut queue = DownloadQueue::new(2);
        let downloader = Arc::new(crate::platforms::magnet::MagnetDownloader::new(Arc::new(
            tokio::sync::Mutex::new(None),
        )));
        queue.enqueue(
            1,
            "magnet:?xt=urn:btih:abc".to_string(),
            "magnet".to_string(),
            "t".to_string(),
            "/tmp".to_string(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            downloader,
            None,
            false,
            None,
            None,
            None,
            None,
            None,
        );
        queue.mark_active(1);
        queue
    }

    #[test]
    fn pausing_a_seeding_torrent_stops_it_and_resuming_goes_back_to_seeding() {
        let mut queue = queue_with_torrent();
        queue.mark_seeding(1, Some("/tmp/t".to_string()), Some(10), Some(7));
        assert!(queue.pause(1));
        assert_eq!(queue.items[0].status, QueueStatus::Paused);
        assert_eq!(queue.items[0].torrent_id, Some(7));
        assert!(queue.resume(1));
        assert_eq!(queue.items[0].status, QueueStatus::Seeding);
    }

    #[test]
    fn pause_all_includes_seeding_torrents() {
        let mut queue = queue_with_torrent();
        queue.mark_seeding(1, None, None, Some(7));
        assert_eq!(queue.pause_all(), vec![(1, Some(7))]);
        assert_eq!(queue.resume_all(), vec![(1, Some(7))]);
        assert_eq!(queue.items[0].status, QueueStatus::Seeding);
    }

    #[test]
    fn a_torrent_paused_while_downloading_resumes_downloading() {
        let mut queue = queue_with_torrent();
        assert!(queue.pause(1));
        assert!(queue.resume(1));
        assert_eq!(queue.items[0].status, QueueStatus::Active);
    }
}

#[cfg(test)]
mod cancelled_run_tests {
    use super::*;

    #[test]
    fn a_run_stopped_while_still_active_is_finished_as_cancelled() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(run_was_cancelled(&QueueStatus::Active, &token));
    }

    #[test]
    fn a_retried_item_is_not_finished_by_the_run_that_was_cancelled() {
        let fresh = CancellationToken::new();
        assert!(!run_was_cancelled(&QueueStatus::Active, &fresh));
    }

    #[test]
    fn an_item_the_user_already_cancelled_or_paused_is_left_alone() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(!run_was_cancelled(&QueueStatus::Paused, &token));
        assert!(!run_was_cancelled(
            &QueueStatus::Error {
                message: "Cancelled".to_string(),
                retryable: false,
            },
            &token
        ));
    }

    #[tokio::test]
    async fn a_cancelled_run_stops_waiting_for_a_busy_host() {
        let host = "cancelled-run-tests.invalid";
        let mut held = Vec::new();
        for _ in 0..4 {
            held.push(crate::core::host_limiter::acquire(host).await);
        }
        let token = CancellationToken::new();
        let waiter = tokio::spawn({
            let token = token.clone();
            async move { host_slot_unless_cancelled(host, &token).await.is_none() }
        });
        token.cancel();
        let gave_up = tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
            .await
            .expect("the cancelled run kept waiting for the host slot")
            .expect("waiter task");
        assert!(gave_up);
        drop(held);
    }

    #[tokio::test]
    async fn a_live_run_gets_the_host_slot() {
        let token = CancellationToken::new();
        assert!(
            host_slot_unless_cancelled("cancelled-run-free.invalid", &token)
                .await
                .is_some()
        );
    }
}
