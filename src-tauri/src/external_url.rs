use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::core::queue::{self, emit_queue_state_from_state};
use crate::platforms::Platform;
use crate::storage::config;
use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalUrlEvent {
    pub url: String,
    pub source: String,
    pub action: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueUrlOutcome {
    Queued,
    AlreadyQueued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalUrlAction {
    Queued,
    Prefill,
    AlreadyQueued,
}

impl ExternalUrlAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Prefill => "prefill",
            Self::AlreadyQueued => "already-queued",
        }
    }
}

pub fn is_external_url(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }

    if trimmed.starts_with("magnet:") || trimmed.starts_with("p2p:") {
        return true;
    }

    url::Url::parse(trimmed)
        .map(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or(false)
}

pub async fn queue_url_with_defaults(
    app: &AppHandle,
    url: String,
    from_hotkey: bool,
) -> Result<QueueUrlOutcome, String> {
    let state = app.state::<AppState>();
    let settings = config::load_settings(app);
    let download_queue = state.download_queue.clone();

    {
        let mut q = download_queue.lock().await;
        q.max_concurrent = settings.advanced.max_concurrent_downloads.max(1);
        q.stagger_delay_ms = settings.advanced.stagger_delay_ms;
        if q.has_url(&url) {
            return Ok(QueueUrlOutcome::AlreadyQueued);
        }
    }

    let downloader = state
        .registry
        .find_platform(&url)
        .ok_or_else(|| "No downloader available for this URL".to_string())?;

    let platform = Platform::from_url(&url);
    let platform_name = platform
        .map(|p| p.to_string())
        .unwrap_or_else(|| "generic".to_string());

    let download_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let ytdlp_path = crate::core::ytdlp::find_ytdlp_cached().await;

    if platform_name == "youtube" || platform_name == "generic" {
        let url_clone = url.clone();
        let downloader_clone = downloader.clone();
        let platform_clone = platform_name.clone();
        let ytdlp_clone = ytdlp_path.clone();
        tokio::spawn(async move {
            queue::prefetch_info(
                &url_clone,
                &*downloader_clone,
                &platform_clone,
                ytdlp_clone.as_deref(),
            )
            .await;
        });
    }

    let output_dir = settings
        .download
        .default_output_dir
        .to_string_lossy()
        .to_string();

    {
        let mut q = download_queue.lock().await;
        q.enqueue(
            download_id,
            url.clone(),
            platform_name,
            url.clone(),
            output_dir,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            downloader,
            ytdlp_path,
            from_hotkey,
        );

        let next_ids = q.next_queued_ids();
        for nid in &next_ids {
            q.mark_active(*nid);
        }
        let state = q.get_state();
        drop(q);
        emit_queue_state_from_state(app, state);
    }

    let q_clone = download_queue.clone();
    let app_clone = app.clone();
    tokio::spawn(async move {
        let ids_to_start = {
            let q = q_clone.lock().await;
            q.items
                .iter()
                .filter(|i| i.status == queue::QueueStatus::Active)
                .filter(|i| i.id == download_id)
                .map(|i| i.id)
                .collect::<Vec<_>>()
        };

        let stagger = {
            let q = q_clone.lock().await;
            q.stagger_delay_ms
        };

        for (i, nid) in ids_to_start.into_iter().enumerate() {
            if i > 0 && stagger > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(stagger)).await;
            }
            let app_handle = app_clone.clone();
            let queue_handle = q_clone.clone();
            tokio::spawn(async move {
                queue::spawn_download(app_handle, queue_handle, nid).await;
            });
        }
    });

    Ok(QueueUrlOutcome::Queued)
}

pub async fn handle_external_url(
    app: &AppHandle,
    url: String,
    source: &str,
) -> Result<ExternalUrlAction, String> {
    if !is_external_url(&url) {
        return Err("Invalid external URL".to_string());
    }

    let settings = config::load_settings(app);
    let can_queue_directly = !settings.download.always_ask_path
        && has_valid_output_dir(&settings.download.default_output_dir);

    let action = if can_queue_directly {
        match queue_url_with_defaults(app, url.clone(), false).await? {
            QueueUrlOutcome::Queued => ExternalUrlAction::Queued,
            QueueUrlOutcome::AlreadyQueued => ExternalUrlAction::AlreadyQueued,
        }
    } else {
        crate::tray::show_window(app);
        ExternalUrlAction::Prefill
    };

    if action != ExternalUrlAction::AlreadyQueued {
        push_or_emit_event(
            app,
            ExternalUrlEvent {
                url,
                source: source.to_string(),
                action: action.as_str().to_string(),
            },
        )
        .await;
    }

    Ok(action)
}

pub async fn register_frontend(app: &AppHandle) -> Vec<ExternalUrlEvent> {
    let state = app.state::<AppState>();

    {
        let mut ready = state.frontend_ready.lock().await;
        *ready = true;
    }

    let mut pending = state.pending_external_events.lock().await;
    std::mem::take(&mut *pending)
}

pub fn find_external_url_arg<I, S>(args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .map(|arg| arg.as_ref().trim().to_string())
        .find(|arg| is_external_url(arg))
}

async fn push_or_emit_event(app: &AppHandle, event: ExternalUrlEvent) {
    let state = app.state::<AppState>();
    let ready = {
        let ready_guard = state.frontend_ready.lock().await;
        *ready_guard
    };

    if ready {
        let _ = app.emit("external-url", &event);
    } else {
        let mut pending = state.pending_external_events.lock().await;
        pending.push(event);
    }
}

fn has_valid_output_dir(path: &PathBuf) -> bool {
    !path.as_os_str().is_empty() && path.is_dir()
}
