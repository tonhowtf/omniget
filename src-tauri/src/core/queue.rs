use std::sync::Arc;

use serde::Serialize;
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::core::ffmpeg::{self, MetadataEmbed};
use crate::models::media::MediaInfo;
use crate::platforms::traits::PlatformDownloader;
use crate::storage::config;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum QueueStatus {
    Queued,
    Active,
    Paused,
    Complete { success: bool },
    Error { message: String },
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
    pub eta_seconds: Option<f64>,
    pub file_path: Option<String>,
    pub file_size_bytes: Option<u64>,
    pub file_count: Option<u32>,
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
    pub percent: f64,
    pub speed_bytes_per_sec: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub eta_seconds: Option<f64>,
    pub file_path: Option<String>,
    pub file_size_bytes: Option<u64>,
    pub file_count: Option<u32>,
    pub media_info: Option<MediaInfo>,
    pub downloader: Arc<dyn PlatformDownloader>,
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
            eta_seconds: self.eta_seconds,
            file_path: self.file_path.clone(),
            file_size_bytes: self.file_size_bytes,
            file_count: self.file_count,
        }
    }
}

pub struct DownloadQueue {
    pub items: Vec<QueueItem>,
    pub max_concurrent: u32,
    pub stagger_delay_ms: u64,
}

impl DownloadQueue {
    pub fn new(max_concurrent: u32) -> Self {
        Self {
            items: Vec::new(),
            max_concurrent,
            stagger_delay_ms: 150,
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
        media_info: Option<MediaInfo>,
        total_bytes: Option<u64>,
        file_count: Option<u32>,
        downloader: Arc<dyn PlatformDownloader>,
    ) {
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
            percent: 0.0,
            speed_bytes_per_sec: 0.0,
            downloaded_bytes: 0,
            total_bytes,
            eta_seconds: None,
            file_path: None,
            file_size_bytes: None,
            file_count,
            media_info,
            downloader,
        };
        self.items.push(item);
    }

    pub fn active_count(&self) -> u32 {
        self.items
            .iter()
            .filter(|i| i.status == QueueStatus::Active)
            .count() as u32
    }

    pub fn next_queued_ids(&self) -> Vec<u64> {
        let slots = self.max_concurrent.saturating_sub(self.active_count()) as usize;
        self.items
            .iter()
            .filter(|i| i.status == QueueStatus::Queued)
            .take(slots)
            .map(|i| i.id)
            .collect()
    }

    pub fn mark_active(&mut self, id: u64) {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
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
            if success {
                item.status = QueueStatus::Complete { success: true };
                item.percent = 100.0;
            } else {
                item.status = QueueStatus::Error {
                    message: error.unwrap_or_default(),
                };
            }
            item.file_path = file_path;
            item.file_size_bytes = file_size_bytes;
            item.speed_bytes_per_sec = 0.0;
            item.eta_seconds = None;
        }
    }

    pub fn update_progress(
        &mut self,
        id: u64,
        percent: f64,
        speed: f64,
        downloaded: u64,
        total: Option<u64>,
        eta: Option<f64>,
    ) {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            item.percent = percent;
            item.speed_bytes_per_sec = speed;
            item.downloaded_bytes = downloaded;
            if let Some(t) = total {
                item.total_bytes = Some(t);
            }
            item.eta_seconds = eta;
        }
    }

    pub fn pause(&mut self, id: u64) -> bool {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            if item.status == QueueStatus::Active {
                item.cancel_token.cancel();
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
                item.status = QueueStatus::Queued;
                item.cancel_token = CancellationToken::new();
                return true;
            }
        }
        false
    }

    pub fn cancel(&mut self, id: u64) -> bool {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            match &item.status {
                QueueStatus::Active => {
                    item.cancel_token.cancel();
                    item.status = QueueStatus::Error {
                        message: "Cancelled".to_string(),
                    };
                    item.speed_bytes_per_sec = 0.0;
                    item.eta_seconds = None;
                    return true;
                }
                QueueStatus::Queued | QueueStatus::Paused => {
                    item.status = QueueStatus::Error {
                        message: "Cancelled".to_string(),
                    };
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    pub fn retry(&mut self, id: u64) -> bool {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            if matches!(item.status, QueueStatus::Error { .. }) {
                item.status = QueueStatus::Queued;
                item.cancel_token = CancellationToken::new();
                item.percent = 0.0;
                item.speed_bytes_per_sec = 0.0;
                item.downloaded_bytes = 0;
                item.eta_seconds = None;
                item.file_path = None;
                item.file_size_bytes = None;
                return true;
            }
        }
        false
    }

    pub fn remove(&mut self, id: u64) -> bool {
        if let Some(pos) = self.items.iter().position(|i| i.id == id) {
            let item = &self.items[pos];
            if item.status == QueueStatus::Active {
                item.cancel_token.cancel();
            }
            self.items.remove(pos);
            return true;
        }
        false
    }

    pub fn clear_finished(&mut self) {
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
                    QueueStatus::Queued | QueueStatus::Active | QueueStatus::Paused
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

pub fn emit_queue_state(app: &tauri::AppHandle, queue: &DownloadQueue) {
    let state = queue.get_state();
    let _ = app.emit("queue-state-update", &state);
    crate::tray::update_active_count(app, queue.active_count());
}

pub fn spawn_download(
    app: tauri::AppHandle,
    queue: Arc<tokio::sync::Mutex<DownloadQueue>>,
    item_id: u64,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
    spawn_download_inner(app, queue, item_id).await;
    })
}

async fn spawn_download_inner(
    app: tauri::AppHandle,
    queue: Arc<tokio::sync::Mutex<DownloadQueue>>,
    item_id: u64,
) {
    let (url, output_dir, download_mode, quality, format_id, referer, cancel_token, media_info, platform_name, downloader) = {
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
            item.cancel_token.clone(),
            item.media_info.clone(),
            item.platform.clone(),
            item.downloader.clone(),
        )
    };

    let info = match media_info {
        Some(i) => i,
        None => match downloader.get_media_info(&url).await {
            Ok(i) => i,
            Err(e) => {
                {
                    let mut q = queue.lock().await;
                    q.mark_complete(item_id, false, Some(e.to_string()), None, None);
                    emit_queue_state(&app, &q);
                }
                try_start_next(app, queue).await;
                return;
            }
        },
    };

    {
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
        }
        emit_queue_state(&app, &q);
    }

    let settings = config::load_settings(&app);
    let tmpl = settings.download.filename_template.clone();
    let mut final_output_dir = std::path::PathBuf::from(&output_dir);
    if settings.download.organize_by_platform {
        final_output_dir = final_output_dir.join(&platform_name);
    }
    let opts = crate::models::media::DownloadOptions {
        quality: Some(quality.unwrap_or(settings.download.video_quality.clone())),
        output_dir: final_output_dir,
        filename_template: Some(tmpl),
        download_subtitles: false,
        download_mode,
        format_id,
        referer,
        cancel_token: cancel_token.clone(),
        concurrent_fragments: settings.advanced.concurrent_fragments,
    };

    let total_bytes = info.file_size_bytes;
    let (tx, mut rx) = mpsc::channel::<f64>(32);

    let app_progress = app.clone();
    let queue_progress = queue.clone();
    let progress_forwarder = tokio::spawn(async move {
        let start_time = std::time::Instant::now();
        let mut last_bytes: u64 = 0;
        let mut last_time = std::time::Instant::now();
        let mut throttle = ProgressThrottle::new(150);
        let mut current_speed: f64 = 0.0;

        while let Some(percent) = rx.recv().await {
            if !throttle.should_emit() && percent < 100.0 {
                continue;
            }

            let now = std::time::Instant::now();
            let downloaded_bytes = total_bytes
                .map(|total| (percent / 100.0 * total as f64) as u64)
                .unwrap_or(0);

            if total_bytes.is_some() && downloaded_bytes > last_bytes {
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

            let elapsed = now.duration_since(start_time).as_secs_f64();
            let eta_seconds = if percent > 0.0 && elapsed > 2.0 {
                let remaining = elapsed * (100.0 - percent) / percent;
                if remaining.is_finite() && remaining >= 0.0 {
                    Some(remaining)
                } else {
                    None
                }
            } else {
                None
            };

            last_bytes = downloaded_bytes;
            last_time = now;

            let (state, active_count) = {
                let mut q = queue_progress.lock().await;
                q.update_progress(
                    item_id,
                    percent,
                    current_speed,
                    downloaded_bytes,
                    total_bytes,
                    eta_seconds,
                );
                (q.get_state(), q.active_count())
            };
            let _ = app_progress.emit("queue-state-update", &state);
            crate::tray::update_active_count(&app_progress, active_count);
        }
    });

    let result = tokio::select! {
        r = downloader.download(&info, &opts, tx) => r,
        _ = cancel_token.cancelled() => {
            Err(anyhow::anyhow!("Download cancelado"))
        }
    };

    let _ = progress_forwarder.await;

    {
        let mut q = queue.lock().await;
        let was_paused = q
            .items
            .iter()
            .find(|i| i.id == item_id)
            .map(|i| i.status == QueueStatus::Paused)
            .unwrap_or(false);

        if was_paused {
            emit_queue_state(&app, &q);
            drop(q);
            try_start_next(app, queue).await;
            return;
        }

        match result {
            Ok(dl) => {
                drop(q);

                if settings.download.embed_metadata && ffmpeg::is_ffmpeg_available().await {
                    let metadata = MetadataEmbed {
                        title: Some(info.title.clone()),
                        artist: Some(info.author.clone()),
                        thumbnail_url: info.thumbnail_url.clone(),
                        ..Default::default()
                    };
                    let client = reqwest::Client::new();
                    if let Err(e) = ffmpeg::embed_metadata(
                        &dl.file_path,
                        &metadata,
                        settings.download.embed_thumbnail,
                        &client,
                    )
                    .await
                    {
                        tracing::warn!("Metadata embed failed for '{}': {}", info.title, e);
                    }
                }

                let mut q = queue.lock().await;
                q.mark_complete(
                    item_id,
                    true,
                    None,
                    Some(dl.file_path.to_string_lossy().to_string()),
                    Some(dl.file_size_bytes),
                );
                emit_queue_state(&app, &q);
            }
            Err(e) => {
                let err_msg = e.to_string();
                tracing::error!("Download error '{}': {}", platform_name, err_msg);
                q.mark_complete(item_id, false, Some(err_msg), None, None);
                emit_queue_state(&app, &q);
            }
        }
    }

    try_start_next(app, queue).await;
}

pub async fn try_start_next(app: tauri::AppHandle, queue: Arc<tokio::sync::Mutex<DownloadQueue>>) {
    let (next_ids, stagger) = {
        let mut q = queue.lock().await;
        let ids = q.next_queued_ids();
        for nid in &ids {
            q.mark_active(*nid);
        }
        if !ids.is_empty() {
            emit_queue_state(&app, &q);
        }
        (ids, q.stagger_delay_ms)
    };

    for (i, nid) in next_ids.into_iter().enumerate() {
        if i > 0 && stagger > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(stagger)).await;
        }
        let app_c = app.clone();
        let queue_c = queue.clone();
        tokio::spawn(async move {
            spawn_download(app_c, queue_c, nid).await;
        });
    }
}
