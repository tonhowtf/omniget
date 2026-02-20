use tauri::{Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::core::queue::{self, emit_queue_state};
use crate::platforms::Platform;
use crate::storage::config;
use crate::AppState;

pub fn register_from_settings(app: &tauri::AppHandle) {
    let settings = config::load_settings(app);
    if !settings.download.hotkey_enabled {
        return;
    }

    let binding = &settings.download.hotkey_binding;
    match binding.parse::<Shortcut>() {
        Ok(shortcut) => {
            if let Err(e) = app.global_shortcut().register(shortcut) {
                tracing::warn!("Failed to register hotkey '{}': {}", binding, e);
            } else {
                tracing::info!("[hotkey] registered: {}", binding);
            }
        }
        Err(e) => {
            tracing::warn!("Invalid hotkey binding '{}': {:?}", binding, e);
        }
    }
}

pub fn on_hotkey_pressed(app: &tauri::AppHandle) {
    let text = match app.clipboard().read_text() {
        Ok(t) => t,
        Err(_) => return,
    };

    let text = text.trim().to_string();
    if text.is_empty() || (!text.starts_with("http://") && !text.starts_with("https://")) {
        return;
    }

    if url::Url::parse(&text).is_err() {
        return;
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        enqueue_from_clipboard(&app, text).await;
    });
}

async fn enqueue_from_clipboard(app: &tauri::AppHandle, url: String) {
    let state = app.state::<AppState>();
    let settings = config::load_settings(app);
    let download_queue = state.download_queue.clone();

    {
        let mut q = download_queue.lock().await;
        q.max_concurrent = settings.advanced.max_concurrent_downloads.max(1);
        q.stagger_delay_ms = settings.advanced.stagger_delay_ms;
        if q.has_url(&url) {
            return;
        }
    }

    let downloader = match state.registry.find_platform(&url) {
        Some(d) => d,
        None => return,
    };

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
        );

        let next_ids = q.next_queued_ids();
        for nid in &next_ids {
            q.mark_active(*nid);
        }
        emit_queue_state(app, &q);
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
            let a = app_clone.clone();
            let qc = q_clone.clone();
            tokio::spawn(async move {
                queue::spawn_download(a, qc, nid).await;
            });
        }
    });

    let _ = app.emit(
        "hotkey-download-queued",
        serde_json::json!({ "url": url }),
    );
}
