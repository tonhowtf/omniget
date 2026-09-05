//! Tools que mexem com a área de trabalho: autoclicker, ditado, gravar tela e
//! a ponte com o VoiceStudio. Aqui também moram os atalhos globais dessas
//! tools (`hotkeys.json` em `<app_data>/tools`), disparados por `hotkey.rs`.

use std::collections::HashMap;

use omniget_core::core::tools::{autoclick, dictation, screen_record, voicestudio};
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use super::{err, progress};

// ── Atalhos globais das tools ──────────────────────────────────────────

fn hotkeys_file() -> Option<std::path::PathBuf> {
    omniget_core::core::tools::tools_dir().map(|d| d.join("hotkeys.json"))
}

pub fn tool_hotkeys() -> HashMap<String, String> {
    hotkeys_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_hotkeys(map: &HashMap<String, String>) -> Result<(), String> {
    let p = hotkeys_file().ok_or("sem pasta de dados")?;
    std::fs::create_dir_all(p.parent().unwrap()).map_err(err)?;
    std::fs::write(p, serde_json::to_string_pretty(map).map_err(err)?).map_err(err)
}

/// Registra os atalhos salvos (chamado no arranque e a cada `reregister`).
pub fn register_tool_hotkeys(app: &tauri::AppHandle) {
    for (action, binding) in tool_hotkeys() {
        match binding.parse::<Shortcut>() {
            Ok(s) => {
                if let Err(e) = app.global_shortcut().register(s) {
                    tracing::warn!(
                        "[tools] atalho {} ({}) nao registrou: {}",
                        binding,
                        action,
                        e
                    );
                }
            }
            Err(e) => tracing::warn!("[tools] atalho invalido {}: {}", binding, e),
        }
    }
}

/// Qual ação de tool este atalho dispara, se alguma.
pub fn action_for(shortcut: &Shortcut) -> Option<String> {
    tool_hotkeys()
        .into_iter()
        .find(|(_, b)| {
            b.parse::<Shortcut>()
                .map(|s| s == *shortcut)
                .unwrap_or(false)
        })
        .map(|(a, _)| a)
}

pub fn on_hotkey(app: &tauri::AppHandle, action: &str) {
    match action {
        "autoclick" => match autoclick::toggle() {
            Ok(running) => {
                let _ = app.emit("tool-autoclick", serde_json::json!({ "running": running }));
            }
            Err(e) => {
                let _ = app.emit(
                    "tool-autoclick",
                    serde_json::json!({ "running": false, "error": e.to_string() }),
                );
            }
        },
        "dictation" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let r = if dictation::is_recording() {
                    dictation_finish(&app).await.map(|_| ())
                } else {
                    dictation::start(progress(&app)).await.map_err(err)
                };
                if let Err(e) = r {
                    let _ = app.emit(
                        "tool-dictation",
                        serde_json::json!({ "phase": "idle", "error": e }),
                    );
                }
            });
        }
        "record" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let st = screen_record::state();
                let r = if st.running && st.replay {
                    screen_record::save_replay()
                        .await
                        .map(|p| serde_json::json!({ "saved": p }))
                } else if st.running {
                    screen_record::stop()
                        .await
                        .map(|s| serde_json::to_value(s).unwrap_or_default())
                } else {
                    Err(anyhow::anyhow!("nenhuma gravacao ativa"))
                };
                let _ = app.emit(
                    "tool-record",
                    r.map_err(|e| e.to_string())
                        .unwrap_or_else(|e| serde_json::json!({ "error": e })),
                );
            });
        }
        _ => {}
    }
}

#[tauri::command]
pub fn tool_hotkeys_get() -> HashMap<String, String> {
    tool_hotkeys()
}

/// Define (ou apaga, com `binding` vazio) o atalho de uma ação.
#[tauri::command]
pub fn tool_hotkey_set(
    app: tauri::AppHandle,
    action: String,
    binding: String,
) -> Result<HashMap<String, String>, String> {
    let mut map = tool_hotkeys();
    if let Some(old) = map.get(&action) {
        if let Ok(s) = old.parse::<Shortcut>() {
            let _ = app.global_shortcut().unregister(s);
        }
    }
    let binding = binding.trim().to_string();
    if binding.is_empty() {
        map.remove(&action);
    } else {
        let s: Shortcut = binding
            .parse()
            .map_err(|e| format!("atalho invalido: {}", e))?;
        if map.values().any(|b| b == &binding) {
            return Err("este atalho ja esta em uso por outra tool".into());
        }
        app.global_shortcut()
            .register(s)
            .map_err(|e| format!("nao registrou o atalho: {}", e))?;
        map.insert(action, binding);
    }
    save_hotkeys(&map)?;
    Ok(map)
}

// ── Autoclicker ────────────────────────────────────────────────────────

#[tauri::command]
pub fn tool_autoclick_start(
    opts: autoclick::ClickOptions,
) -> Result<autoclick::ClickState, String> {
    autoclick::start(opts).map_err(err)?;
    Ok(autoclick::state())
}

#[tauri::command]
pub fn tool_autoclick_stop() -> autoclick::ClickState {
    autoclick::stop();
    autoclick::state()
}

#[tauri::command]
pub fn tool_autoclick_state() -> autoclick::ClickState {
    autoclick::state()
}

#[tauri::command]
pub fn tool_autoclick_mouse() -> Option<(i32, i32)> {
    autoclick::mouse_position()
}

// ── Ditado ─────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn tool_dictation_devices() -> Vec<dictation::AudioDevice> {
    dictation::devices().await
}

#[tauri::command]
pub fn tool_dictation_options() -> dictation::DictationOptions {
    dictation::options()
}

#[tauri::command]
pub fn tool_dictation_set_options(
    opts: dictation::DictationOptions,
) -> dictation::DictationOptions {
    dictation::set_options(opts);
    dictation::options()
}

#[tauri::command]
pub fn tool_dictation_state() -> dictation::DictationState {
    dictation::state()
}

#[tauri::command]
pub async fn tool_dictation_start(
    app: tauri::AppHandle,
) -> Result<dictation::DictationState, String> {
    dictation::start(progress(&app)).await.map_err(err)?;
    let _ = app.emit(
        "tool-dictation",
        serde_json::json!({ "phase": "recording" }),
    );
    Ok(dictation::state())
}

#[derive(Serialize, Deserialize)]
pub struct DictationOut {
    pub text: String,
    pub delivered: String,
}

async fn dictation_finish(app: &tauri::AppHandle) -> Result<DictationOut, String> {
    let _ = app.emit(
        "tool-dictation",
        serde_json::json!({ "phase": "transcribing" }),
    );
    let text = dictation::stop(progress(app)).await.map_err(err)?;
    let opts = dictation::options();
    let delivered = match opts.output.as_str() {
        "paste" => {
            app.clipboard().write_text(text.clone()).map_err(err)?;
            let r = tokio::task::spawn_blocking(dictation::press_paste)
                .await
                .map_err(err)?;
            match r {
                Ok(_) => "paste",
                Err(_) => "clipboard",
            }
        }
        "clipboard" => {
            app.clipboard().write_text(text.clone()).map_err(err)?;
            "clipboard"
        }
        _ => {
            let t = text.clone();
            let r = tokio::task::spawn_blocking(move || dictation::type_text(&t))
                .await
                .map_err(err)?;
            match r {
                Ok(_) => "type",
                Err(e) => {
                    // Sem permissão para digitar: pelo menos deixa no clipboard.
                    app.clipboard().write_text(text.clone()).map_err(err)?;
                    let _ = app.emit(
                        "tool-dictation",
                        serde_json::json!({ "phase": "idle", "warning": e.to_string() }),
                    );
                    "clipboard"
                }
            }
        }
    };
    let _ = app.emit(
        "tool-dictation",
        serde_json::json!({ "phase": "idle", "text": text, "delivered": delivered }),
    );
    Ok(DictationOut {
        text,
        delivered: delivered.into(),
    })
}

#[tauri::command]
pub async fn tool_dictation_stop(app: tauri::AppHandle) -> Result<DictationOut, String> {
    dictation_finish(&app).await
}

// ── Gravar tela ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn tool_record_sources() -> Vec<screen_record::Source> {
    screen_record::sources().await
}

#[tauri::command]
pub fn tool_record_state() -> screen_record::RecordState {
    screen_record::state()
}

#[tauri::command]
pub async fn tool_record_start(
    opts: screen_record::RecordOptions,
) -> Result<screen_record::RecordState, String> {
    screen_record::start(opts).await.map_err(err)
}

#[tauri::command]
pub async fn tool_record_stop() -> Result<screen_record::RecordState, String> {
    screen_record::stop().await.map_err(err)
}

#[tauri::command]
pub async fn tool_record_save_replay() -> Result<String, String> {
    screen_record::save_replay().await.map_err(err)
}

// ── VoiceStudio ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn tool_vs_status(base_url: Option<String>) -> voicestudio::VsStatus {
    voicestudio::status(base_url.as_deref().unwrap_or("")).await
}

#[tauri::command]
pub async fn tool_vs_launch() -> Result<(), String> {
    voicestudio::launch().await.map_err(err)
}

#[tauri::command]
pub async fn tool_vs_clone(
    app: tauri::AppHandle,
    opts: voicestudio::CloneOptions,
) -> Result<voicestudio::SpeechResult, String> {
    voicestudio::clone_speak(opts, progress(&app))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn tool_vs_design(
    app: tauri::AppHandle,
    opts: voicestudio::DesignOptions,
) -> Result<voicestudio::DesignResult, String> {
    voicestudio::design_speak(opts, progress(&app))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn tool_vs_isolate(
    app: tauri::AppHandle,
    base_url: Option<String>,
    input: String,
    output_dir: Option<String>,
    instrumental: Option<bool>,
) -> Result<Vec<String>, String> {
    voicestudio::isolate(
        base_url.as_deref().unwrap_or(""),
        &input,
        output_dir.as_deref().unwrap_or(""),
        instrumental.unwrap_or(true),
        progress(&app),
    )
    .await
    .map_err(err)
}
