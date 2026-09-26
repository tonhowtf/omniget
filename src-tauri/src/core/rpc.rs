use std::collections::HashMap;
use std::sync::Mutex;

use discord_rich_presence::activity::{Activity, Assets, Button, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use once_cell::sync::Lazy;
use serde_json::{json, Value};

pub use omniget_core::models::settings::RpcSettings;

const GITHUB_BUTTON_LABEL: &str = "View on GitHub";
const GITHUB_BUTTON_URL: &str = "https://github.com/tonhowtf/omniget";
const APP_LARGE_TEXT: &str = "omniget";

const SOURCE_PRIORITIES: &[&str] = &["focus", "music", "video", "course", "reading"];

fn priority_of(source: &str) -> usize {
    SOURCE_PRIORITIES
        .iter()
        .position(|s| *s == source)
        .unwrap_or(SOURCE_PRIORITIES.len())
}

#[derive(Debug, Clone)]
struct SourceActivity {
    source: String,
    details: String,
    state: String,
    duration_secs: i64,
    position_secs: i64,
    paused: bool,
    large_image_key: Option<String>,
    timestamp: i64,
}

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
    activities: HashMap<String, SourceActivity>,
    idle_stats: IdleStats,
    last_displayed_source: Option<String>,
}

static RPC: Lazy<Mutex<RpcState>> = Lazy::new(|| {
    Mutex::new(RpcState {
        client: None,
        connected: false,
        connected_app_id: String::new(),
        session_start_ts: 0,
        activities: HashMap::new(),
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

fn truncate_for_discord(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max - 1).collect();
    format!("{}…", truncated)
}

fn format_position_duration(position: i64, duration: i64) -> Option<String> {
    if duration <= 0 {
        return None;
    }
    let p = position.max(0);
    let fmt = |sec: i64| -> String {
        let m = sec / 60;
        let s = sec % 60;
        format!("{}:{:02}", m, s)
    };
    Some(format!("{} / {}", fmt(p), fmt(duration)))
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

fn select_top_activity(state: &RpcState) -> Option<SourceActivity> {
    state
        .activities
        .values()
        .min_by_key(|a| (priority_of(&a.source), -a.timestamp))
        .cloned()
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

fn apply_top_activity(default_image: &str) -> bool {
    let mut state = match RPC.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    let session_start = state.session_start_ts;
    let idle_stats = state.idle_stats;
    let top = select_top_activity(&state);
    let client = match state.client.as_mut() {
        Some(c) => c,
        None => return false,
    };

    let (details_str, state_str, source_tag, img_key_owned) = match top {
        Some(a) => {
            let mut details = if a.details.is_empty() {
                "—".to_string()
            } else {
                truncate_for_discord(&a.details, 64)
            };
            if details.chars().count() < 2 {
                details.push(' ');
                details.push(' ');
            }
            let scrub = if !a.paused {
                format_position_duration(a.position_secs, a.duration_secs)
            } else {
                None
            };
            let mut state_text = if a.state.is_empty() {
                String::new()
            } else {
                truncate_for_discord(&a.state, 64)
            };
            if let Some(s) = scrub {
                if state_text.is_empty() {
                    state_text = s;
                } else {
                    state_text = format!("{} · {}", state_text, s);
                }
            }
            if state_text.is_empty() {
                state_text = "—".into();
            }
            let img = a
                .large_image_key
                .clone()
                .unwrap_or_else(|| default_image.to_string());
            (details, state_text, a.source.clone(), img)
        }
        None => {
            let details = "omniget".to_string();
            let state_text = idle_state_text(idle_stats);
            (
                details,
                state_text,
                "idle".to_string(),
                default_image.to_string(),
            )
        }
    };

    let assets = Assets::new()
        .large_image(&img_key_owned)
        .large_text(APP_LARGE_TEXT);
    let buttons = vec![Button::new(GITHUB_BUTTON_LABEL, GITHUB_BUTTON_URL)];

    let start = if session_start > 0 {
        session_start
    } else {
        chrono::Utc::now().timestamp()
    };
    let timestamps = Timestamps::new().start(start);

    let activity = Activity::new()
        .details(&details_str)
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
    state.last_displayed_source = Some(source_tag);
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

pub async fn set_presence_source(
    settings: RpcSettings,
    source: String,
    details: String,
    state_text: String,
    duration_secs: i64,
    position_secs: i64,
    paused: bool,
    large_image_key: Option<String>,
) -> Result<Value, String> {
    if !settings.enabled || settings.app_id.is_empty() {
        let _ = tokio::task::spawn_blocking(|| {
            if let Ok(mut s) = RPC.lock() {
                if s.connected {
                    disconnect_locked(&mut s);
                }
                s.activities.clear();
            }
        })
        .await;
        return Ok(json!({ "ok": false, "reason": "disabled" }));
    }

    let source_owned = if source.trim().is_empty() {
        "music".to_string()
    } else {
        source.trim().to_string()
    };
    let app_id = settings.app_id.clone();
    let default_img = settings.large_image_key.clone();
    let activity_record = SourceActivity {
        source: source_owned.clone(),
        details,
        state: state_text,
        duration_secs,
        position_secs,
        paused,
        large_image_key,
        timestamp: chrono::Utc::now().timestamp_millis(),
    };

    let outcome = tokio::task::spawn_blocking(move || -> bool {
        if !ensure_connected_with(&app_id) {
            return false;
        }
        if let Ok(mut state) = RPC.lock() {
            state
                .activities
                .insert(source_owned.clone(), activity_record);
        }
        apply_top_activity(&default_img)
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok(json!({ "ok": outcome }))
}

pub async fn clear_source(settings: RpcSettings, source: String) -> Result<Value, String> {
    let default_img = settings.large_image_key.clone();
    let app_id = settings.app_id.clone();
    let enabled = settings.enabled;
    let source_owned = if source.trim().is_empty() {
        "music".to_string()
    } else {
        source.trim().to_string()
    };
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(mut state) = RPC.lock() {
            state.activities.remove(&source_owned);
        }
        if !enabled || app_id.is_empty() {
            return;
        }
        if !ensure_connected_with(&app_id) {
            return;
        }
        let _ = apply_top_activity(&default_img);
    })
    .await;
    Ok(json!({ "ok": true }))
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
        apply_top_activity(&default_img)
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
            let _ = apply_top_activity(&default_img);
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_order_focus_first() {
        assert!(priority_of("focus") < priority_of("music"));
        assert!(priority_of("music") < priority_of("video"));
        assert!(priority_of("video") < priority_of("course"));
        assert!(priority_of("course") < priority_of("reading"));
    }

    #[test]
    fn unknown_source_lowest_priority() {
        let unknown = priority_of("xyz");
        let known = priority_of("music");
        assert!(unknown > known);
    }

    #[test]
    fn format_position_duration_basic() {
        assert_eq!(
            format_position_duration(83, 245),
            Some("1:23 / 4:05".into())
        );
    }

    #[test]
    fn format_position_zero_duration_none() {
        assert_eq!(format_position_duration(50, 0), None);
    }

    #[test]
    fn count_short_thousands() {
        assert_eq!(format_count_short(999), "999");
        assert_eq!(format_count_short(1_200), "1.2k");
        assert_eq!(format_count_short(150_000), "150k");
        assert_eq!(format_count_short(1_500_000), "1.5M");
    }

    #[test]
    fn bytes_short_units() {
        assert_eq!(format_bytes_short(0), "0 B");
        assert_eq!(format_bytes_short(1500), "1.5 KB");
        assert_eq!(format_bytes_short(1_500_000_000), "1.4 GB");
    }

    #[test]
    fn idle_text_empty() {
        let stats = IdleStats::default();
        assert_eq!(idle_state_text(stats), "Ready");
    }

    #[test]
    fn idle_text_with_count_only() {
        let stats = IdleStats {
            downloads_count: 12,
            total_bytes: 0,
        };
        assert_eq!(idle_state_text(stats), "12 downloads");
    }

    #[test]
    fn idle_text_with_count_and_bytes() {
        let stats = IdleStats {
            downloads_count: 1234,
            total_bytes: 1_500_000_000,
        };
        assert_eq!(idle_state_text(stats), "1.2k downloads · 1.4 GB");
    }
}
