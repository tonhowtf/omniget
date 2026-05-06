use crate::hotkey;
use crate::models::settings::AppSettings;
use crate::storage::config;

#[tauri::command]
pub fn get_settings(app: tauri::AppHandle) -> Result<AppSettings, String> {
    Ok(config::load_settings(&app))
}

#[tauri::command]
pub fn update_settings(app: tauri::AppHandle, partial: String) -> Result<AppSettings, String> {
    let mut current = config::load_settings(&app);
    let old_hotkey_enabled = current.download.hotkey_enabled;
    let old_hotkey_binding = current.download.hotkey_binding.clone();
    let old_clip_hotkey_enabled = current.download.clip_hotkey_enabled;
    let old_clip_hotkey_binding = current.download.clip_hotkey_binding.clone();
    let old_music_hotkey_enabled = current.download.music_hotkey_enabled;
    let old_music_hotkey_binding = current.download.music_hotkey_binding.clone();
    let old_start_with_system = current.start_with_system;
    let old_rpc = current.rpc.clone();

    let patch: serde_json::Value =
        serde_json::from_str(&partial).map_err(|e| format!("Invalid JSON: {}", e))?;
    let mut current_val =
        serde_json::to_value(&current).map_err(|e| format!("Serialize: {}", e))?;
    merge_json(&mut current_val, &patch);
    current = serde_json::from_value(current_val).map_err(|e| format!("Deserialize: {}", e))?;
    config::save_settings(&app, &current).map_err(|e| format!("Save: {}", e))?;

    crate::core::http_client::init_proxy(current.proxy.clone());
    crate::core::http_fetcher::set_global_max_concurrent_segments(
        current.advanced.max_concurrent_segments as usize,
    );

    if old_hotkey_enabled != current.download.hotkey_enabled
        || old_hotkey_binding != current.download.hotkey_binding
        || old_clip_hotkey_enabled != current.download.clip_hotkey_enabled
        || old_clip_hotkey_binding != current.download.clip_hotkey_binding
        || old_music_hotkey_enabled != current.download.music_hotkey_enabled
        || old_music_hotkey_binding != current.download.music_hotkey_binding
    {
        hotkey::reregister(&app);
    }

    if old_start_with_system != current.start_with_system {
        crate::commands::autostart::apply_autostart(&app, current.start_with_system)?;
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
    let _ = crate::commands::autostart::apply_autostart(&app, defaults.start_with_system);
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
    config::save_settings(&app, &current).map_err(|e| format!("Save: {}", e))?;
    get_bridge_info(app)
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
