use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

pub trait TelegramHost: Send + Sync + 'static {
    fn emit(&self, event: &str, payload: serde_json::Value);
    fn data_dir(&self) -> PathBuf;
    fn settings(&self) -> serde_json::Value;
    fn save_settings(&self, settings: serde_json::Value);
    fn default_output_dir(&self) -> PathBuf;
}

static HOST: OnceLock<Arc<dyn TelegramHost>> = OnceLock::new();

pub fn install(host: Arc<dyn TelegramHost>) {
    let _ = HOST.set(host);
}

pub fn get() -> Option<Arc<dyn TelegramHost>> {
    HOST.get().cloned()
}

pub fn data_dir() -> PathBuf {
    let dir = get().map(|h| h.data_dir()).unwrap_or_else(|| {
        crate::core::paths::app_data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("telegram")
    });
    let _ = std::fs::create_dir_all(&dir);
    dir
}
