use crate::hotkey;
use crate::models::settings::AppSettings;
use crate::storage::config;

/// The login item exists when the person asked for it, or for agents in
/// the background (3.2: they start hidden at login).
pub fn autostart_wanted(s: &AppSettings) -> bool {
    s.start_with_system || s.run_in_background
}

/// "Run agents in the background" and "quit when the window is closed"
/// contradict each other (3.2). The one just turned on wins; turned on
/// together, the agents keep running.
pub fn reconcile_close(s: &mut AppSettings, old_background: bool, old_exit_on_close: bool) {
    if s.run_in_background && s.exit_on_close {
        if old_background && !old_exit_on_close {
            s.run_in_background = false;
        } else {
            s.exit_on_close = false;
        }
    }
}

#[tauri::command]
pub fn get_settings(app: tauri::AppHandle) -> Result<AppSettings, String> {
    Ok(config::load_settings(&app))
}

/// Locales with a file in `src/lib/i18n/` (the list of `i18n/index.ts`).
/// Anything else would show English while claiming to be applied.
pub const APP_LANGUAGES: &[&str] = &[
    "en", "ru", "el", "pt", "zh", "zh-TW", "ja", "it", "fr", "es", "fa", "lo",
];

#[tauri::command]
pub fn update_settings(app: tauri::AppHandle, partial: String) -> Result<AppSettings, String> {
    let mut current = config::load_settings(&app);
    let old_hotkey_enabled = current.download.hotkey_enabled;
    let old_hotkey_binding = current.download.hotkey_binding.clone();
    let old_clip_hotkey_enabled = current.download.clip_hotkey_enabled;
    let old_clip_hotkey_binding = current.download.clip_hotkey_binding.clone();
    let old_music_hotkey_enabled = current.download.music_hotkey_enabled;
    let old_music_hotkey_binding = current.download.music_hotkey_binding.clone();
    let old_ptt_key = current.omnidisc.voice.ptt_key.clone();
    let old_start_with_system = current.start_with_system;
    let old_background = current.run_in_background;
    let old_exit_on_close = current.exit_on_close;
    let old_autostart = autostart_wanted(&current);
    let old_rpc = current.rpc.clone();
    let old_language = current.appearance.language.clone();

    let patch: serde_json::Value =
        serde_json::from_str(&partial).map_err(|e| format!("Invalid JSON: {}", e))?;
    let mut current_val =
        serde_json::to_value(&current).map_err(|e| format!("Serialize: {}", e))?;
    merge_json(&mut current_val, &patch);
    current = serde_json::from_value(current_val).map_err(|e| format!("Deserialize: {}", e))?;
    if current.appearance.language != old_language
        && !APP_LANGUAGES.contains(&current.appearance.language.as_str())
    {
        return Err(format!(
            "SETTING_INVALID: appearance.language '{}' has no translation; one of {}",
            current.appearance.language,
            APP_LANGUAGES.join(", ")
        ));
    }
    reconcile_close(&mut current, old_background, old_exit_on_close);
    current.canvas.normalize();
    config::save_settings(&app, &current).map_err(|e| format!("Save: {}", e))?;
    if current.appearance.language != old_language {
        // Every window (the canvas, Omni's window, the pet) follows live.
        crate::canvas::companion::on_language(&app, &current.appearance.language);
    }

    crate::core::http_client::init_proxy(current.proxy.clone());
    crate::commands::washmybelly::sync_settings(&current.washmybelly);
    crate::core::http_fetcher::set_global_max_concurrent_segments(
        current.advanced.max_concurrent_segments as usize,
    );

    if old_hotkey_enabled != current.download.hotkey_enabled
        || old_hotkey_binding != current.download.hotkey_binding
        || old_clip_hotkey_enabled != current.download.clip_hotkey_enabled
        || old_clip_hotkey_binding != current.download.clip_hotkey_binding
        || old_music_hotkey_enabled != current.download.music_hotkey_enabled
        || old_music_hotkey_binding != current.download.music_hotkey_binding
        || old_ptt_key != current.omnidisc.voice.ptt_key
    {
        hotkey::reregister(&app);
    }

    if old_start_with_system != current.start_with_system
        || old_autostart != autostart_wanted(&current)
    {
        crate::commands::autostart::apply_autostart(&app, autostart_wanted(&current))?;
    }

    if old_rpc.enabled != current.rpc.enabled
        || old_rpc.app_id != current.rpc.app_id
        || old_rpc.large_image_key != current.rpc.large_image_key
    {
        let new_rpc = current.rpc.clone();
        let prev_app_id = old_rpc.app_id.clone();
        tauri::async_runtime::spawn(async move {
            crate::core::rpc::handle_settings_changed(new_rpc, prev_app_id).await;
        });
    }

    Ok(current)
}

#[tauri::command]
pub fn reset_settings(app: tauri::AppHandle) -> Result<AppSettings, String> {
    let defaults = AppSettings::default();
    config::save_settings(&app, &defaults).map_err(|e| format!("Save: {}", e))?;
    hotkey::reregister(&app);
    let _ = crate::commands::autostart::apply_autostart(&app, autostart_wanted(&defaults));
    Ok(defaults)
}

#[tauri::command]
pub fn mark_onboarding_complete(app: tauri::AppHandle) -> Result<(), String> {
    let mut current = config::load_settings(&app);
    current.onboarding_completed = true;
    config::save_settings(&app, &current).map_err(|e| format!("Save: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn mark_legal_acknowledged(app: tauri::AppHandle) -> Result<(), String> {
    let mut current = config::load_settings(&app);
    current.legal_acknowledged = true;
    config::save_settings(&app, &current).map_err(|e| format!("Save: {}", e))?;
    Ok(())
}

#[derive(serde::Serialize)]
pub struct BridgeInfo {
    pub enabled: bool,
    pub port: u16,
    pub token: String,
    pub url: String,
}

/// Returns the localhost bridge connection info for the Settings UI.
///
/// `port == 0` means the bridge has not finished binding yet (the user can
/// retry shortly). `token` may be empty until the first run completes.
#[tauri::command]
pub fn get_bridge_info(app: tauri::AppHandle) -> Result<BridgeInfo, String> {
    let settings = config::load_settings(&app);
    let url = if settings.bridge.port == 0 {
        String::new()
    } else {
        crate::local_bridge::build_pairing_url(settings.bridge.port)
    };
    Ok(BridgeInfo {
        enabled: settings.bridge.enabled,
        port: settings.bridge.port,
        token: settings.bridge.token,
        url,
    })
}

/// Forces a fresh token to be generated. Used by the Settings UI when the
/// user wants to revoke a previously-paired browser.
#[tauri::command]
pub fn rotate_bridge_token(app: tauri::AppHandle) -> Result<BridgeInfo, String> {
    let mut current = config::load_settings(&app);
    current.bridge.token = crate::local_bridge::generate_token();
    omniget_core::core::llm::cli_runtime::session::register_secret_value(&current.bridge.token);
    config::save_settings(&app, &current).map_err(|e| format!("Save: {}", e))?;
    get_bridge_info(app)
}

#[derive(serde::Serialize)]
pub struct BridgePairStatus {
    pub ok: bool,
    pub window_secs: i64,
    pub port: u16,
    pub enabled: bool,
}

/// Opens a short, single-use, user-consented pairing window so an installed
/// browser extension can fetch the bearer token from `GET /v1/pair` without
/// the user copy-pasting it.
#[tauri::command]
pub fn bridge_open_pairing(app: tauri::AppHandle) -> Result<BridgePairStatus, String> {
    let settings = config::load_settings(&app);
    if !settings.bridge.enabled {
        return Ok(BridgePairStatus {
            ok: false,
            window_secs: 0,
            port: settings.bridge.port,
            enabled: false,
        });
    }
    let window_secs = crate::local_bridge::open_pairing_window();
    Ok(BridgePairStatus {
        ok: true,
        window_secs,
        port: settings.bridge.port,
        enabled: true,
    })
}

fn merge_json(base: &mut serde_json::Value, patch: &serde_json::Value) {
    if let (Some(base_obj), Some(patch_obj)) = (base.as_object_mut(), patch.as_object()) {
        for (key, value) in patch_obj {
            if value.is_object() && base_obj.get(key).is_some_and(|v| v.is_object()) {
                merge_json(base_obj.get_mut(key).unwrap(), value);
            } else {
                base_obj.insert(key.clone(), value.clone());
            }
        }
    }
}

#[cfg(test)]
mod background_tests {
    use super::*;

    #[test]
    fn the_switch_turned_on_last_wins() {
        let mut s = AppSettings {
            run_in_background: true,
            exit_on_close: true,
            ..AppSettings::default()
        };
        // Background was on, quit-on-close just turned on.
        reconcile_close(&mut s, true, false);
        assert!(s.exit_on_close && !s.run_in_background);

        let mut s = AppSettings {
            run_in_background: true,
            exit_on_close: true,
            ..AppSettings::default()
        };
        // Quit-on-close was on, background just turned on.
        reconcile_close(&mut s, false, true);
        assert!(s.run_in_background && !s.exit_on_close);

        let mut s = AppSettings {
            run_in_background: true,
            exit_on_close: true,
            ..AppSettings::default()
        };
        reconcile_close(&mut s, false, false);
        assert!(
            s.run_in_background && !s.exit_on_close,
            "together: agents win"
        );
    }

    #[test]
    fn background_agents_need_the_login_item() {
        let mut s = AppSettings::default();
        assert!(!autostart_wanted(&s));
        s.run_in_background = true;
        assert!(autostart_wanted(&s));
        s.run_in_background = false;
        s.start_with_system = true;
        assert!(autostart_wanted(&s));
    }
}
