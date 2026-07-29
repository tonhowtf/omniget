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

    if settings.download.hotkey_enabled {
        register_one(app, &settings.download.hotkey_binding, "download");
    }
    if settings.download.clip_hotkey_enabled {
        register_one(app, &settings.download.clip_hotkey_binding, "clip");
    }
    if settings.download.music_hotkey_enabled {
        register_one(app, &settings.download.music_hotkey_binding, "music");
    }
}

fn register_one(app: &tauri::AppHandle, binding: &str, label: &str) {
    match binding.parse::<Shortcut>() {
        Ok(shortcut) => {
            if let Err(e) = app.global_shortcut().register(shortcut) {
                tracing::warn!("[hotkey] register {} '{}' failed: {}", label, binding, e);
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
                tracing::info!("[hotkey] registered {}: {}", label, binding);
            }
        }
        Err(e) => {
            tracing::warn!("[hotkey] invalid {} binding '{}': {:?}", label, binding, e);
        }
    }
}

pub fn on_hotkey_pressed(app: &tauri::AppHandle, shortcut: &Shortcut) {
    let settings = config::load_settings(app);

    let download_match = matches_binding(shortcut, &settings.download.hotkey_binding);
    let clip_match = matches_binding(shortcut, &settings.download.clip_hotkey_binding);
    let music_match = matches_binding(shortcut, &settings.download.music_hotkey_binding);

    if settings.download.clip_hotkey_enabled && clip_match {
        let _ = app.emit("clip-hotkey-pressed", serde_json::json!({}));
        return;
    }

    if settings.download.music_hotkey_enabled && music_match {
        handle_music_clipboard(app);
        return;
    }

    if settings.download.hotkey_enabled && download_match {
        handle_download_clipboard(app);
    }
}

fn matches_binding(pressed: &Shortcut, binding: &str) -> bool {
    binding
        .parse::<Shortcut>()
        .map(|s| s == *pressed)
        .unwrap_or(false)
}

/// Resultado de um acionamento do atalho global, sempre reportado à interface.
///
/// Antes, cada caminho de falha fazia `return` mudo: quem apertava o atalho com
/// o clipboard errado via o app não fazer absolutamente nada e concluía que o
/// atalho estava quebrado (issue #198). O evento de sucesso existia mas ninguém
/// escutava, então nem o caminho feliz dava sinal.
fn emit_hotkey_result(app: &tauri::AppHandle, outcome: &str, url: Option<&str>) {
    let _ = app.emit(
        "hotkey-download-result",
        serde_json::json!({ "outcome": outcome, "url": url }),
    );
}

fn handle_download_clipboard(app: &tauri::AppHandle) {
    let text = match app.clipboard().read_text() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("[hotkey] nao foi possivel ler o clipboard: {}", e);
            emit_hotkey_result(app, "clipboard_error", None);
            return;
        }
    };

    let text = text.trim().to_string();
    if text.is_empty() || (!text.starts_with("http://") && !text.starts_with("https://")) {
        emit_hotkey_result(app, "not_a_url", None);
        return;
    }

    if url::Url::parse(&text).is_err() {
        emit_hotkey_result(app, "not_a_url", None);
        return;
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        enqueue_from_clipboard(&app, text).await;
    });
}

async fn enqueue_from_clipboard(app: &tauri::AppHandle, url: String) {
    use crate::external_url::QueueUrlOutcome;
    match crate::external_url::queue_url_with_defaults(app, url.clone(), true, None).await {
        Ok(QueueUrlOutcome::Queued) => emit_hotkey_result(app, "queued", Some(&url)),
        Ok(QueueUrlOutcome::AlreadyQueued) => emit_hotkey_result(app, "already_queued", Some(&url)),
        Err(e) => {
            tracing::warn!("[hotkey] enfileiramento recusado: {}", e);
            emit_hotkey_result(app, "unsupported", Some(&url));
        }
    }
}

fn handle_music_clipboard(app: &tauri::AppHandle) {
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
        if matches!(
            crate::external_url::queue_url_with_defaults(
                &app,
                text.clone(),
                true,
                Some("audio".to_string()),
            )
            .await,
            Ok(crate::external_url::QueueUrlOutcome::Queued)
        ) {
            let _ = app.emit("hotkey-download-queued", serde_json::json!({ "url": text }));
        }
    });
}
