use tauri::Emitter;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::storage::config;

pub fn reregister(app: &tauri::AppHandle) {
    if let Err(e) = app.global_shortcut().unregister_all() {
        tracing::warn!("Failed to unregister hotkeys: {}", e);
    }
    #[cfg(windows)]
    forget_held_ptt();
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
    let ptt = settings.omnidisc.voice.ptt_key.trim();
    if settings.omnidisc.enabled && !ptt.is_empty() {
        register_one(app, ptt, "omnidisc-ptt");
    }
    crate::commands::tools::desktop::register_tool_hotkeys(app);
}

pub fn handle_ptt(app: &tauri::AppHandle, shortcut: &Shortcut, pressed: bool) -> bool {
    let settings = config::load_settings(app);
    let ptt = settings.omnidisc.voice.ptt_key.trim();
    if ptt.is_empty() || !matches_binding(shortcut, ptt) {
        return false;
    }
    #[cfg(windows)]
    tame_key_repeat(app, *shortcut, pressed);
    crate::commands::omnidisc::voice::ptt_from_hotkey(app, pressed);
    true
}

/// Windows repeats `WM_HOTKEY` for as long as the key is down, and the
/// global-shortcut backend answers every repeat by spawning a thread that
/// busy-polls that key until it comes back up. A few seconds of push-to-talk
/// would therefore pile up dozens of spinning threads — a call that heats the
/// machine while you talk. Dropping the registration for the duration of the
/// hold stops the repeat at the source, and the release still arrives, because
/// the thread the first press spawned is already watching the key.
#[cfg(windows)]
fn tame_key_repeat(app: &tauri::AppHandle, shortcut: Shortcut, pressed: bool) {
    use std::sync::atomic::Ordering;

    if !pressed {
        restore_ptt_binding(app, shortcut);
        return;
    }
    if PTT_HELD.swap(true, Ordering::AcqRel) {
        return;
    }
    let generation = PTT_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    if let Err(e) = app.global_shortcut().unregister(shortcut) {
        tracing::debug!("[hotkey] could not pause the push-to-talk binding: {}", e);
    }
    // A release that never lands would leave the microphone open and the key
    // unbound, so a hold gets an upper bound it can recover from.
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(PTT_MAX_HOLD);
        if PTT_GENERATION.load(Ordering::Acquire) != generation {
            return;
        }
        tracing::warn!("[hotkey] push-to-talk stayed down too long; releasing it");
        crate::commands::omnidisc::voice::ptt_from_hotkey(&app, false);
        restore_ptt_binding(&app, shortcut);
    });
}

#[cfg(windows)]
fn restore_ptt_binding(app: &tauri::AppHandle, shortcut: Shortcut) {
    use std::sync::atomic::Ordering;

    if !PTT_HELD.swap(false, Ordering::AcqRel) {
        return;
    }
    PTT_GENERATION.fetch_add(1, Ordering::AcqRel);
    if let Err(e) = app.global_shortcut().register(shortcut) {
        tracing::warn!("[hotkey] push-to-talk could not be re-armed: {}", e);
    }
}

#[cfg(windows)]
fn forget_held_ptt() {
    use std::sync::atomic::Ordering;

    PTT_HELD.store(false, Ordering::Release);
    PTT_GENERATION.fetch_add(1, Ordering::AcqRel);
}

#[cfg(windows)]
static PTT_HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(windows)]
static PTT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(windows)]
const PTT_MAX_HOLD: std::time::Duration = std::time::Duration::from_secs(30);

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
                #[cfg(not(target_os = "macos"))]
                {
                    tracing::warn!(
                        "[hotkey] '{}' is probably already claimed by another application; \
                        the user has to pick a different combination",
                        binding
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
    // Atalhos das tools (autoclicker, ditado, replay) vêm antes dos do app.
    if let Some(action) = crate::commands::tools::desktop::action_for(shortcut) {
        crate::commands::tools::desktop::on_hotkey(app, &action);
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
