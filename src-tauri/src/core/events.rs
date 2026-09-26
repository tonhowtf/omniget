use omniget_core::core::events::{EventEmitter, QueueItemInfo, QueueItemProgress};
use tauri::Emitter;

#[derive(Clone)]
pub struct TauriEventEmitter {
    app: tauri::AppHandle,
}

impl TauriEventEmitter {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl EventEmitter for TauriEventEmitter {
    fn emit_queue_state(&self, items: &[QueueItemInfo]) {
        let redacted: Vec<QueueItemInfo> = items
            .iter()
            .cloned()
            .map(|mut i| {
                i.url = crate::core::flight_recorder::redact_url(&i.url);
                if let omniget_core::core::events::QueueStatus::Error { message } = &mut i.status {
                    *message = crate::core::flight_recorder::redact_urls(message);
                }
                i
            })
            .collect();
        let _ = self.app.emit("queue-state-update", &redacted);
        let active_count = items
            .iter()
            .filter(|i| i.status == omniget_core::core::events::QueueStatus::Active)
            .count() as u32;
        crate::tray::update_active_count(&self.app, active_count);
    }

    fn emit_progress(&self, progress: &QueueItemProgress) {
        let _ = self.app.emit("queue-item-progress", progress);
    }
}
