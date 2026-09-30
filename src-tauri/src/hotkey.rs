use tauri::Emitter;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use std::sync::Mutex;

use crate::models::settings::AppSettings;
use crate::storage::config;

/// The shortcuts this module registered, by label. Only these are ever
/// unregistered here, so a shortcut owned by another feature survives.
static REGISTERED: Mutex<Vec<(&'static str, Shortcut)>> = Mutex::new(Vec::new());

pub fn reregister(app: &tauri::AppHandle) {
    register_from_settings(app);
}

pub fn register_from_settings(app: &tauri::AppHandle) {
    let settings = config::load_settings(app);
    if let Err(e) = apply_bindings(app, &settings) {
        tracing::warn!("[hotkey] kept the previous shortcuts: {}", e);
    }
    // Omni's window (⌘⇧O) lives outside the download settings.
    crate::canvas::companion::register_shortcut(app);
}

/// The `shortcuts.registration` effect. Every enabled binding is parsed and
/// every new one registered before any old one is released; if one fails,
/// the new ones are rolled back and the previous bindings stay in place.
pub fn apply_bindings(app: &tauri::AppHandle, settings: &AppSettings) -> Result<(), String> {
    let d = &settings.download;
    let mut desired: Vec<(&'static str, Shortcut)> = Vec::new();
    for (enabled, binding, label) in [
        (d.hotkey_enabled, &d.hotkey_binding, "download"),
        (d.clip_hotkey_enabled, &d.clip_hotkey_binding, "clip"),
        (d.music_hotkey_enabled, &d.music_hotkey_binding, "music"),
    ] {
        if !enabled {
            continue;
        }
        let shortcut = binding.parse::<Shortcut>().map_err(|e| {
            tracing::warn!("[hotkey] invalid {} binding '{}': {:?}", label, binding, e);
            format!("invalid {label} binding")
        })?;
        desired.push((label, shortcut));
    }

    let mut registered = REGISTERED.lock().unwrap_or_else(|p| p.into_inner());
    let manager = app.global_shortcut();
    let held = |s: &Shortcut, list: &[(&str, Shortcut)]| list.iter().any(|(_, o)| o == s);
    let mut added: Vec<Shortcut> = Vec::new();
    for (label, shortcut) in &desired {
        if held(shortcut, &registered) || added.contains(shortcut) {
            continue;
        }
        if let Err(e) = manager.register(*shortcut) {
            for s in &added {
                let _ = manager.unregister(*s);
            }
            report_register_failure(app, label, &e.to_string());
            return Err(format!("{label} shortcut unavailable"));
        }
        tracing::info!("[hotkey] registered {}", label);
        added.push(*shortcut);
    }
    for (_, old) in registered.iter() {
        if !held(old, &desired) {
            if let Err(e) = manager.unregister(*old) {
                tracing::warn!("Failed to unregister hotkey: {}", e);
            }
        }
    }
    *registered = desired;
    Ok(())
}

fn report_register_failure(app: &tauri::AppHandle, label: &str, error: &str) {
    tracing::warn!("[hotkey] register {} failed: {}", label, error);
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
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        tracing::warn!(
            "[hotkey] the {} shortcut is probably already claimed by another application; \
            the user has to pick a different combination",
            label
        );
    }
}

pub fn on_hotkey_pressed(app: &tauri::AppHandle, shortcut: &Shortcut) {
    if crate::canvas::companion::on_shortcut(app, shortcut) {
        return;
    }
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
