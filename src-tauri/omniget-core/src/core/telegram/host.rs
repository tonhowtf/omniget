//! What the Telegram engine needs from whoever embeds it (the desktop app,
//! the CLI): events, folders and settings. Replaces the old plugin SDK host.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

pub trait TelegramHost: Send + Sync + 'static {
    /// Sends an event to the UI (`telegram-*`, progress and completion).
    fn emit(&self, event: &str, payload: serde_json::Value);
    /// `<app data>/telegram`, created on demand.
    fn data_dir(&self) -> PathBuf;
    /// The `telegram` settings object (download folder, threads, …).
    fn settings(&self) -> serde_json::Value;
    fn save_settings(&self, settings: serde_json::Value);
    /// Where downloads go when the person has not picked a folder.
    fn default_output_dir(&self) -> PathBuf;
    fn toast(&self, _kind: &str, _message: &str) {}
}

static HOST: OnceLock<Arc<dyn TelegramHost>> = OnceLock::new();

pub fn install(host: Arc<dyn TelegramHost>) {
    let _ = HOST.set(host);
}

pub fn get() -> Option<Arc<dyn TelegramHost>> {
    HOST.get().cloned()
}

/// Data folder even before a host is installed (CLI, tests).
pub fn data_dir() -> PathBuf {
    let dir = get().map(|h| h.data_dir()).unwrap_or_else(|| {
        crate::core::paths::app_data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("telegram")
    });
    let _ = std::fs::create_dir_all(&dir);
    dir
}
