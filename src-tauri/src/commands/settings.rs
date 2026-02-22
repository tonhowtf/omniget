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

    let patch: serde_json::Value =
        serde_json::from_str(&partial).map_err(|e| format!("Invalid JSON: {}", e))?;
    let mut current_val =
        serde_json::to_value(&current).map_err(|e| format!("Serialize: {}", e))?;
    merge_json(&mut current_val, &patch);
    current =
        serde_json::from_value(current_val).map_err(|e| format!("Deserialize: {}", e))?;
    config::save_settings(&app, &current).map_err(|e| format!("Save: {}", e))?;

    if old_hotkey_enabled != current.download.hotkey_enabled
        || old_hotkey_binding != current.download.hotkey_binding
    {
        hotkey::reregister(&app);
    }

    Ok(current)
}

#[tauri::command]
pub fn reset_settings(app: tauri::AppHandle) -> Result<AppSettings, String> {
    let defaults = AppSettings::default();
    config::save_settings(&app, &defaults).map_err(|e| format!("Save: {}", e))?;
    hotkey::reregister(&app);
    Ok(defaults)
}

#[tauri::command]
pub fn mark_onboarding_complete(app: tauri::AppHandle) -> Result<(), String> {
    let mut current = config::load_settings(&app);
    current.onboarding_completed = true;
    config::save_settings(&app, &current).map_err(|e| format!("Save: {}", e))?;
    Ok(())
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
