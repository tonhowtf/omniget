use std::sync::{Mutex, OnceLock};

static GUARD: OnceLock<Mutex<Option<keepawake::KeepAwake>>> = OnceLock::new();

fn cell() -> &'static Mutex<Option<keepawake::KeepAwake>> {
    GUARD.get_or_init(|| Mutex::new(None))
}

pub fn sync(active: bool) {
    let Ok(mut held) = cell().lock() else {
        return;
    };
    if active && held.is_none() {
        let enabled = crate::storage::config::load_settings_standalone()
            .advanced
            .prevent_sleep;
        if !enabled {
            return;
        }
        match keepawake::Builder::default()
            .idle(true)
            .sleep(true)
            .app_name("OmniGet")
            .reason("Active downloads")
            .create()
        {
            Ok(g) => {
                *held = Some(g);
                tracing::debug!("[awake] sleep prevention engaged");
            }
            Err(e) => tracing::warn!("[awake] could not prevent sleep: {}", e),
        }
    } else if !active && held.is_some() {
        *held = None;
        tracing::debug!("[awake] sleep prevention released");
    }
}
