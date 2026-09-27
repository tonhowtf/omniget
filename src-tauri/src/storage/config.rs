use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::models::settings::AppSettings;

const STORE_PATH: &str = "settings.json";
const STORE_KEY: &str = "app_settings";

/// Absolute path of the settings store, anchored to the app data dir that
/// honors OMNIGET_DATA_DIR. A relative store path resolves against Tauri's
/// own AppData dir, which ignores the portable-mode override — the store
/// writers and the standalone readers below would then use different files.
fn store_path() -> std::path::PathBuf {
    match crate::core::paths::app_data_dir() {
        Some(dir) => dir.join(STORE_PATH),
        None => std::path::PathBuf::from(STORE_PATH),
    }
}

pub fn load_settings(app: &AppHandle) -> AppSettings {
    let store = match app.store(store_path()) {
        Ok(s) => s,
        Err(_) => return AppSettings::default(),
    };

    match store.get(STORE_KEY) {
        Some(val) => {
            let mut settings = parse_settings(&val);
            if settings.migrate() {
                let _ = save_settings(app, &settings);
            }
            settings
        }
        None => AppSettings::default(),
    }
}

pub fn load_settings_standalone() -> AppSettings {
    let data_dir = match crate::core::paths::app_data_dir() {
        Some(d) => d,
        None => return AppSettings::default(),
    };

    let store_path = data_dir.join(STORE_PATH);
    let content = match std::fs::read_to_string(&store_path) {
        Ok(c) => c,
        Err(_) => return AppSettings::default(),
    };

    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return AppSettings::default(),
    };

    match json.get(STORE_KEY) {
        Some(val) => parse_settings(val),
        None => AppSettings::default(),
    }
}

/// Settings as stored, read by this version. A file written by another
/// version (a field it dropped, or one of a different type) used to fail as a
/// whole and cost the user every setting, the extension token included. Now
/// the stored values are laid over the defaults, so only the fields that do
/// not fit fall back.
fn parse_settings(val: &serde_json::Value) -> AppSettings {
    if let Ok(settings) = serde_json::from_value::<AppSettings>(val.clone()) {
        return settings;
    }
    let mut merged = serde_json::to_value(AppSettings::default()).unwrap_or_default();
    overlay(&mut merged, val);
    serde_json::from_value(merged).unwrap_or_default()
}

/// Copies `saved` into `base` where the JSON kinds agree; a value of another
/// kind keeps the default. Keys this version does not know are kept (serde
/// ignores them).
fn overlay(base: &mut serde_json::Value, saved: &serde_json::Value) {
    use serde_json::Value;
    if let (Value::Object(b), Value::Object(s)) = (&mut *base, saved) {
        for (k, v) in s {
            match b.get_mut(k) {
                Some(slot) => overlay(slot, v),
                None => {
                    b.insert(k.clone(), v.clone());
                }
            }
        }
        return;
    }
    if base.is_null() || std::mem::discriminant(&*base) == std::mem::discriminant(saved) {
        *base = saved.clone();
    }
}

pub fn save_settings(app: &AppHandle, settings: &AppSettings) -> anyhow::Result<()> {
    let store = app.store(store_path())?;
    let val = serde_json::to_value(settings)?;
    store.set(STORE_KEY, val);
    store.save()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored() -> serde_json::Value {
        let mut s = AppSettings::default();
        s.download.video_quality = "1080p".into();
        s.bridge.token = "paired-token".into();
        serde_json::to_value(s).unwrap()
    }

    // Bench 2026-09-26: v0.10.1 read a file written by a newer build (one field
    // dropped) and came up with factory settings and a new extension token.
    #[test]
    fn a_missing_field_keeps_every_other_setting() {
        let mut val = stored();
        val["download"]
            .as_object_mut()
            .unwrap()
            .remove("video_quality");
        let s = parse_settings(&val);
        assert_eq!(s.bridge.token, "paired-token");
        assert_eq!(
            s.download.video_quality,
            AppSettings::default().download.video_quality
        );
    }

    #[test]
    fn a_field_of_another_type_falls_back_alone() {
        let mut val = stored();
        val["download"]["video_quality"] = serde_json::json!(1080);
        val["download"]["field_from_a_newer_version"] = serde_json::json!(true);
        let s = parse_settings(&val);
        assert_eq!(s.bridge.token, "paired-token");
        assert_eq!(
            s.download.video_quality,
            AppSettings::default().download.video_quality
        );
    }

    #[test]
    fn a_valid_file_is_read_as_is() {
        let s = parse_settings(&stored());
        assert_eq!(s.download.video_quality, "1080p");
        assert_eq!(s.bridge.token, "paired-token");
    }
}
