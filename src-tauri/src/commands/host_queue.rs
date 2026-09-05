use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Listener, Manager};
use tokio_util::sync::CancellationToken;

use crate::core::queue::{
    self, emit_queue_state_from_state, kind_from_platform, QueueItem, QueueItemInfo, QueueKind,
    QueueStatus,
};
use crate::platforms::noop::NoopDownloader;
use crate::AppState;

static EXTERNAL_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_external_id() -> u64 {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let bump = EXTERNAL_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    now_ms.wrapping_mul(1000).wrapping_add(bump & 0xFFFF)
}

#[derive(Deserialize)]
pub struct EnqueueExternalArgs {
    pub platform: String,
    pub title: String,
    pub output_path: PathBuf,
    pub total_bytes: Option<u64>,
    pub thumbnail_url: Option<String>,
    pub kind: Option<QueueKind>,
    pub url: Option<String>,
    pub file_name: Option<String>,
}

#[derive(Serialize)]
pub struct EnqueueExternalResult {
    pub queue_id: u64,
}

pub async fn enqueue_external_inner(
    app: &AppHandle,
    state: &AppState,
    args: EnqueueExternalArgs,
    forced_id: Option<u64>,
) -> Result<u64, String> {
    let queue_id = forced_id.unwrap_or_else(next_external_id);
    let kind = args
        .kind
        .unwrap_or_else(|| kind_from_platform(&args.platform));
    let output_dir = args
        .output_path
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let initial_path = args.output_path.to_string_lossy().to_string();

    let item = QueueItem {
        id: queue_id,
        url: args.url.unwrap_or_default(),
        platform: args.platform.clone(),
        title: args.title,
        status: QueueStatus::Active,
        cancel_token: CancellationToken::new(),
        output_dir,
        download_mode: None,
        quality: None,
        format_id: None,
        referer: None,
        extra_headers: None,
        page_url: None,
        user_agent: None,
        percent: 0.0,
        speed_bytes_per_sec: 0.0,
        downloaded_bytes: 0,
        total_bytes: args.total_bytes,
        file_path: Some(initial_path),
        file_size_bytes: args.total_bytes,
        file_count: None,
        media_info: None,
        downloader: Arc::new(NoopDownloader::new()),
        ytdlp_path: None,
        from_hotkey: false,
        torrent_id: None,
        kind: Some(kind),
        external: true,
        thumbnail_url_override: args.thumbnail_url,
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
        phase: None,
        current_stream: None,
        streams_done: Vec::new(),
        planned_formats: None,
        fragment_index: None,
        fragment_count: None,
        started_at_ms: Some(crate::core::queue::now_ms()),
        ytdlp_argv_override: None,
    };

    {
        let mut q = state.download_queue.lock().await;
        q.items.retain(|i| i.id != queue_id);
        q.items.push(item);
    }

    let snapshot: Vec<QueueItemInfo> = {
        let q = state.download_queue.lock().await;
        q.get_state()
    };
    emit_queue_state_from_state(app, snapshot);

    tracing::info!(
        "[host-queue] enqueue_external id={} platform={} kind={:?}",
        queue_id,
        args.platform,
        kind
    );
    Ok(queue_id)
}

#[tauri::command]
pub async fn host_queue_enqueue_external(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    args: EnqueueExternalArgs,
) -> Result<EnqueueExternalResult, String> {
    let queue_id = enqueue_external_inner(&app, &state, args, None).await?;
    Ok(EnqueueExternalResult { queue_id })
}

#[derive(Deserialize)]
pub struct ReportProgressArgs {
    pub queue_id: u64,
    pub percent: f64,
    pub downloaded_bytes: u64,
    pub speed_bytes_per_sec: f64,
}

pub async fn report_progress_inner(
    app: &AppHandle,
    state: &AppState,
    args: ReportProgressArgs,
) -> Result<(), String> {
    let snapshot: Option<Vec<QueueItemInfo>> = {
        let mut q = state.download_queue.lock().await;
        let item = q.items.iter_mut().find(|i| i.id == args.queue_id);
        match item {
            Some(it) => {
                if !it.external {
                    return Err(format!("queue_id {} is not external", args.queue_id));
                }
                it.percent = args.percent.clamp(0.0, 100.0);
                it.downloaded_bytes = args.downloaded_bytes;
                it.speed_bytes_per_sec = args.speed_bytes_per_sec;
                if !matches!(it.status, QueueStatus::Active) {
                    it.status = QueueStatus::Active;
                }
                Some(q.get_state())
            }
            None => None,
        }
    };

    match snapshot {
        Some(state_snap) => {
            emit_queue_state_from_state(app, state_snap);
            Ok(())
        }
        None => Err(format!("queue_id {} not found", args.queue_id)),
    }
}

#[tauri::command]
pub async fn host_queue_report_progress(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    args: ReportProgressArgs,
) -> Result<(), String> {
    report_progress_inner(&app, &state, args).await
}

#[derive(Deserialize)]
pub struct ReportCompleteArgs {
    pub queue_id: u64,
    pub success: bool,
    pub file_path: Option<PathBuf>,
    pub error: Option<String>,
    pub file_size_bytes: Option<u64>,
}

pub async fn report_complete_inner(
    app: &AppHandle,
    state: &AppState,
    args: ReportCompleteArgs,
) -> Result<(), String> {
    let snapshot: Option<Vec<QueueItemInfo>> = {
        let mut q = state.download_queue.lock().await;
        let item = q.items.iter_mut().find(|i| i.id == args.queue_id);
        match item {
            Some(it) => {
                if !it.external {
                    return Err(format!("queue_id {} is not external", args.queue_id));
                }
                if args.success {
                    it.percent = 100.0;
                    if let Some(ref p) = args.file_path {
                        it.file_path = Some(p.to_string_lossy().to_string());
                    }
                    if let Some(sz) = args.file_size_bytes {
                        it.file_size_bytes = Some(sz);
                    }
                    it.status = QueueStatus::Complete { success: true };
                } else {
                    let msg = args
                        .error
                        .clone()
                        .unwrap_or_else(|| "Unknown error".to_string());
                    let retryable = crate::core::queue::is_retryable_error_message(&msg);
                    it.status = QueueStatus::Error {
                        message: msg,
                        retryable,
                    };
                }
                Some(q.get_state())
            }
            None => None,
        }
    };

    match snapshot {
        Some(state_snap) => {
            emit_queue_state_from_state(app, state_snap);
            tracing::info!(
                "[host-queue] report_complete id={} success={} ",
                args.queue_id,
                args.success
            );
            Ok(())
        }
        None => Err(format!("queue_id {} not found", args.queue_id)),
    }
}

#[tauri::command]
pub async fn host_queue_report_complete(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    args: ReportCompleteArgs,
) -> Result<(), String> {
    report_complete_inner(&app, &state, args).await
}

#[derive(Deserialize)]
struct EnqueueEventPayload {
    queue_id: u64,
    #[serde(flatten)]
    args: EnqueueExternalArgs,
}

pub fn register_event_listeners(app: &AppHandle) {
    let app_for_enqueue = app.clone();
    app.listen("host:queue:external:enqueue", move |event| {
        let payload_str = event.payload();
        let parsed: Result<EnqueueEventPayload, _> = serde_json::from_str(payload_str);
        match parsed {
            Ok(p) => {
                let app = app_for_enqueue.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<AppState>();
                    if let Err(e) =
                        enqueue_external_inner(&app, &state, p.args, Some(p.queue_id)).await
                    {
                        tracing::warn!("[host-queue] enqueue listener failed: {}", e);
                    }
                });
            }
            Err(e) => tracing::warn!("[host-queue] enqueue payload parse failed: {}", e),
        }
    });

    let app_for_progress = app.clone();
    app.listen("host:queue:external:progress", move |event| {
        let payload_str = event.payload();
        let parsed: Result<ReportProgressArgs, _> = serde_json::from_str(payload_str);
        match parsed {
            Ok(args) => {
                let app = app_for_progress.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<AppState>();
                    if let Err(e) = report_progress_inner(&app, &state, args).await {
                        tracing::debug!("[host-queue] progress listener failed: {}", e);
                    }
                });
            }
            Err(e) => tracing::warn!("[host-queue] progress payload parse failed: {}", e),
        }
    });

    let app_for_complete = app.clone();
    app.listen("host:queue:external:complete", move |event| {
        let payload_str = event.payload();
        let parsed: Result<ReportCompleteArgs, _> = serde_json::from_str(payload_str);
        match parsed {
            Ok(args) => {
                let app = app_for_complete.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<AppState>();
                    if let Err(e) = report_complete_inner(&app, &state, args).await {
                        tracing::warn!("[host-queue] complete listener failed: {}", e);
                    }
                });
            }
            Err(e) => tracing::warn!("[host-queue] complete payload parse failed: {}", e),
        }
    });

    tracing::info!("[host-queue] registered 3 event listeners (enqueue/progress/complete)");
}

#[allow(dead_code)]
fn _ensure_queue_module_loaded() {
    let _ = queue::ProgressThrottle::new(0);
}
