use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::models::settings::AppSettings;

const STORE_PATH: &str = "settings.json";
const STORE_KEY: &str = "app_settings";

pub fn load_settings(app: &AppHandle) -> AppSettings {
    let store = match app.store(STORE_PATH) {
        Ok(s) => s,
        Err(_) => return AppSettings::default(),
    };

    match store.get(STORE_KEY) {
        Some(val) => serde_json::from_value::<AppSettings>(val.clone()).unwrap_or_default(),
        None => AppSettings::default(),
    }
}

pub fn save_settings(app: &AppHandle, settings: &AppSettings) -> anyhow::Result<()> {
    let store = app.store(STORE_PATH)?;
    let val = serde_json::to_value(settings)?;
    store.set(STORE_KEY, val);
    store.save()?;
    Ok(())
}
