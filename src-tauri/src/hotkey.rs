use tauri::Emitter;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::storage::config;

pub fn reregister(app: &tauri::AppHandle) {
    if let Err(e) = app.global_shortcut().unregister_all() {
        tracing::warn!("Failed to unregister hotkeys: {}", e);
    }
    register_from_settings(app);
}

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
                #[cfg(target_os = "macos")]
                {
                    tracing::warn!(
                        "[hotkey] macOS: Global shortcut registration failed. \
                        The app may need Accessibility permission. \
                        Go to System Settings > Privacy & Security > Accessibility \
                        and enable OmniGet."
                    );
                    let _ = app.emit(
                        "hotkey-permission-error",
                        serde_json::json!({
                            "message": "Global hotkey requires Accessibility permission. Open System Settings > Privacy & Security > Accessibility and enable OmniGet.",
                            "platform": "macos"
                        }),
                    );
                }
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
    if matches!(
        crate::external_url::queue_url_with_defaults(app, url.clone(), true).await,
        Ok(crate::external_url::QueueUrlOutcome::Queued)
    ) {
        let _ = app.emit(
            "hotkey-download-queued",
            serde_json::json!({ "url": url }),
        );
    }
}
