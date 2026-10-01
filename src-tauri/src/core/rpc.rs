use std::sync::Mutex;

use discord_rich_presence::activity::{Activity, Assets, Button, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use once_cell::sync::Lazy;
use serde_json::{json, Value};

pub use omniget_core::models::settings::RpcSettings;

const GITHUB_BUTTON_LABEL: &str = "View on GitHub";
const GITHUB_BUTTON_URL: &str = "https://github.com/tonhowtf/omniget";
const APP_LARGE_TEXT: &str = "omniget";

#[derive(Debug, Clone, Copy, Default)]
struct IdleStats {
    downloads_count: u64,
    total_bytes: u64,
}

struct RpcState {
    client: Option<DiscordIpcClient>,
    connected: bool,
    connected_app_id: String,
    session_start_ts: i64,
    idle_stats: IdleStats,
    last_displayed_source: Option<String>,
}

static RPC: Lazy<Mutex<RpcState>> = Lazy::new(|| {
    Mutex::new(RpcState {
        client: None,
        connected: false,
        connected_app_id: String::new(),
        session_start_ts: 0,
        idle_stats: IdleStats::default(),
        last_displayed_source: None,
    })
});

fn disconnect_locked(state: &mut RpcState) {
    if let Some(client) = state.client.as_mut() {
        let _ = client.clear_activity();
        let _ = client.close();
    }
    state.client = None;
    state.connected = false;
    state.connected_app_id.clear();
    state.last_displayed_source = None;
}

fn ensure_connected_with(app_id: &str) -> bool {
    let mut state = match RPC.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    if state.connected && state.connected_app_id == app_id {
        return true;
    }
    if state.connected_app_id != app_id {
        disconnect_locked(&mut state);
    }
    if state.client.is_none() {
        match DiscordIpcClient::new(app_id) {
            Ok(c) => state.client = Some(c),
            Err(_) => return false,
        }
    }
    if let Some(client) = state.client.as_mut() {
        if client.connect().is_ok() {
            state.connected = true;
            state.connected_app_id = app_id.to_string();
            if state.session_start_ts == 0 {
                state.session_start_ts = chrono::Utc::now().timestamp();
            }
            return true;
        }
    }
    false
}

fn format_bytes_short(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut idx = 0;
    while value >= 1024.0 && idx < units.len() - 1 {
        value /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{} {}", bytes, units[idx])
    } else if value >= 100.0 {
        format!("{:.0} {}", value, units[idx])
    } else {
        format!("{:.1} {}", value, units[idx])
    }
}

fn format_count_short(count: u64) -> String {
    if count < 1_000 {
        count.to_string()
    } else if count < 1_000_000 {
        let v = count as f64 / 1_000.0;
        if v >= 100.0 {
            format!("{:.0}k", v)
        } else {
            format!("{:.1}k", v)
        }
    } else {
        let v = count as f64 / 1_000_000.0;
        format!("{:.1}M", v)
    }
}

fn idle_state_text(stats: IdleStats) -> String {
    if stats.downloads_count == 0 {
        return "Ready".to_string();
    }
    if stats.total_bytes == 0 {
        return format!("{} downloads", format_count_short(stats.downloads_count));
    }
    format!(
        "{} downloads · {}",
        format_count_short(stats.downloads_count),
        format_bytes_short(stats.total_bytes)
    )
}

fn apply_idle_activity(default_image: &str) -> bool {
    let mut state = match RPC.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    let session_start = state.session_start_ts;
    let idle_stats = state.idle_stats;
    let client = match state.client.as_mut() {
        Some(c) => c,
        None => return false,
    };

    let state_str = idle_state_text(idle_stats);

    let assets = Assets::new()
        .large_image(default_image)
        .large_text(APP_LARGE_TEXT);
    let buttons = vec![Button::new(GITHUB_BUTTON_LABEL, GITHUB_BUTTON_URL)];

    let start = if session_start > 0 {
        session_start
    } else {
        chrono::Utc::now().timestamp()
    };
    let timestamps = Timestamps::new().start(start);

    let activity = Activity::new()
        .details("omniget")
        .state(&state_str)
        .assets(assets)
        .buttons(buttons)
        .timestamps(timestamps);

    let result = client.set_activity(activity).is_ok();
    if !result {
        state.connected = false;
        state.last_displayed_source = None;
        return false;
    }
    state.last_displayed_source = Some("idle".to_string());
    true
}

pub async fn test_connection(settings: RpcSettings) -> Result<Value, String> {
    if settings.app_id.is_empty() {
        return Ok(json!({ "ok": false, "reason": "missing_app_id" }));
    }
    let app_id = settings.app_id.clone();
    let img = settings.large_image_key.clone();
    let ok = tokio::task::spawn_blocking(move || -> bool {
        if !ensure_connected_with(&app_id) {
            return false;
        }
        let mut state = match RPC.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let session_start = if state.session_start_ts > 0 {
            state.session_start_ts
        } else {
            chrono::Utc::now().timestamp()
        };
        let client = match state.client.as_mut() {
            Some(c) => c,
            None => return false,
        };
        let assets = Assets::new().large_image(&img).large_text(APP_LARGE_TEXT);
        let buttons = vec![Button::new(GITHUB_BUTTON_LABEL, GITHUB_BUTTON_URL)];
        let activity = Activity::new()
            .details("omniget")
            .state("Testing connection")
            .assets(assets)
            .buttons(buttons)
            .timestamps(Timestamps::new().start(session_start));
        let res = client.set_activity(activity).is_ok();
        state.last_displayed_source = Some("__test__".into());
        res
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": ok }))
}

pub async fn set_idle_stats(
    settings: RpcSettings,
    downloads_count: u64,
    total_bytes: u64,
) -> Result<Value, String> {
    if !settings.enabled || settings.app_id.is_empty() {
        return Ok(json!({ "ok": false, "reason": "disabled" }));
    }
    let app_id = settings.app_id.clone();
    let default_img = settings.large_image_key.clone();
    let outcome = tokio::task::spawn_blocking(move || -> bool {
        if let Ok(mut state) = RPC.lock() {
            state.idle_stats = IdleStats {
                downloads_count,
                total_bytes,
            };
        }
        if !ensure_connected_with(&app_id) {
            return false;
        }
        apply_idle_activity(&default_img)
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": outcome }))
}

pub async fn handle_settings_changed(new_settings: RpcSettings, previous_app_id: String) {
    let id_changed = previous_app_id != new_settings.app_id;
    let disabled = !new_settings.enabled || new_settings.app_id.is_empty();
    if id_changed || disabled {
        let _ = tokio::task::spawn_blocking(|| {
            if let Ok(mut state) = RPC.lock() {
                disconnect_locked(&mut state);
                state.session_start_ts = 0;
            }
        })
        .await;
    }
    if !disabled {
        let app_id = new_settings.app_id.clone();
        let default_img = new_settings.large_image_key.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if !ensure_connected_with(&app_id) {
                return;
            }
            let _ = apply_idle_activity(&default_img);
        })
        .await;
    }
}
