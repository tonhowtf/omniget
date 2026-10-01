use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use omniget_core::core::washmybelly::{self as wmb, plan::Source};
use once_cell::sync::Lazy;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

static SETTINGS_CACHE: Lazy<
    std::sync::Mutex<
        Option<(
            omniget_core::models::settings::WashMyBellySettings,
            std::time::Instant,
        )>,
    >,
> = Lazy::new(|| std::sync::Mutex::new(None));

const SETTINGS_TTL: std::time::Duration = std::time::Duration::from_millis(2000);

fn settings() -> omniget_core::models::settings::WashMyBellySettings {
    if let Ok(cache) = SETTINGS_CACHE.lock() {
        if let Some((settings, at)) = cache.as_ref() {
            if at.elapsed() < SETTINGS_TTL {
                return settings.clone();
            }
        }
    }
    let settings = crate::storage::config::load_settings_standalone().washmybelly;
    sync_settings(&settings);
    if let Ok(mut cache) = SETTINGS_CACHE.lock() {
        *cache = Some((settings.clone(), std::time::Instant::now()));
    }
    settings
}

pub fn sync_settings(s: &omniget_core::models::settings::WashMyBellySettings) {
    wmb::set_settings(wmb::CoreSettings {
        enabled: s.enabled,
        quarantine_days: s.quarantine_days,
        protected_paths_extra: s.protected_paths_extra.clone(),
        perf_tool_path: s.perf_tool_path.clone(),
    });
    wmb::power::set_auto_hold(s.enabled && s.auto_hold_power);
    if let Ok(mut cache) = SETTINGS_CACHE.lock() {
        *cache = None;
    }
}

fn ensure_enabled() -> Result<(), String> {
    if settings().enabled {
        Ok(())
    } else {
        Err("washmybelly is disabled in settings".to_string())
    }
}

pub fn start_background(app: AppHandle) {
    let s = crate::storage::config::load_settings_standalone().washmybelly;
    sync_settings(&s);
    let handle = app.clone();
    wmb::set_progress_sink(Arc::new(move |p| {
        let _ = handle.emit("washmybelly-progress", p);
    }));
    let days = s.quarantine_days.max(1);
    tauri::async_runtime::spawn_blocking(move || {
        wmb::game::recover();
        if let Ok(ctx) = wmb::store::Ctx::app() {
            wmb::quarantine::purge_expired(&ctx, days);
        }
    });
}

#[tauri::command]
pub async fn washmybelly_capabilities(refresh: Option<bool>) -> Result<Value, String> {
    ensure_enabled()?;
    wmb::tools::call_as(
        Source::Ui,
        "wmb_capabilities",
        serde_json::json!({ "refresh": refresh.unwrap_or(false) }),
    )
    .await
}

#[tauri::command]
pub async fn washmybelly_call(tool: String, input: Option<Value>) -> Result<Value, String> {
    ensure_enabled()?;
    if !tool.starts_with("wmb_") {
        return Err(format!("unknown tool: {tool}"));
    }
    wmb::tools::call_as(Source::Ui, &tool, input.unwrap_or(Value::Null)).await
}

#[tauri::command]
pub async fn washmybelly_ui(action: String, input: Option<Value>) -> Result<Value, String> {
    ensure_enabled()?;
    wmb::ui::call(&action, input.unwrap_or(Value::Null)).await
}

static MONITOR_ON: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub async fn washmybelly_monitor_start(app: AppHandle) -> Result<(), String> {
    ensure_enabled()?;
    if MONITOR_ON.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    tauri::async_runtime::spawn(async move {
        while MONITOR_ON.load(Ordering::SeqCst) {
            let tick = tauri::async_runtime::spawn_blocking(wmb::monitor::live_snapshot).await;
            match tick {
                Ok(Ok(v)) => {
                    let _ = app.emit("washmybelly-monitor", v);
                }
                _ => {
                    MONITOR_ON.store(false, Ordering::SeqCst);
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    Ok(())
}

#[tauri::command]
pub fn washmybelly_monitor_stop() {
    MONITOR_ON.store(false, Ordering::SeqCst);
}

#[tauri::command]
pub async fn washmybelly_approvals() -> Result<Vec<Value>, String> {
    tauri::async_runtime::spawn_blocking(|| wmb::approvals::pending(&wmb::store::Ctx::app()?))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn washmybelly_approval_answer(id: String, allow: bool) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        wmb::approvals::answer(&wmb::store::Ctx::app()?, &id, allow)
    })
    .await
    .map_err(|e| e.to_string())?
}
