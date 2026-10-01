use std::sync::{Arc, OnceLock};
use std::time::Duration;

use omniget_core::core::canvas::notes;
use omniget_core::core::canvas::store::{self, CanvasStore, StoreEvent};
use tauri::{AppHandle, Emitter};

pub const EVENT_PATCH: &str = "canvas://patch";
pub const EVENT_WORKSPACES: &str = "canvas://workspaces";
pub const EVENT_NOTE: &str = "canvas://note";

const NOTE_POLL: Duration = Duration::from_secs(2);

pub struct CanvasDocHost {
    store: Arc<CanvasStore>,
    app: OnceLock<AppHandle>,
}

static HOST: OnceLock<CanvasDocHost> = OnceLock::new();

pub fn host() -> &'static CanvasDocHost {
    HOST.get_or_init(|| CanvasDocHost {
        store: store::shared(),
        app: OnceLock::new(),
    })
}

impl CanvasDocHost {
    pub fn store(&self) -> &Arc<CanvasStore> {
        &self.store
    }

    pub fn bind(&'static self, app: &AppHandle) -> &'static Self {
        if self.app.set(app.clone()).is_ok() {
            crate::commands::canvas::workspace_meta::install();
            super::activity::install(app, &self.store);
            if let Some(dir) = omniget_core::core::paths::app_data_dir() {
                use tauri::Manager;
                let _ = app
                    .asset_protocol_scope()
                    .allow_directory(dir.join("canvas"), true);
            }
            let handle = app.clone();
            let weak = Arc::downgrade(&self.store);
            self.store.subscribe(move |ev| match ev {
                StoreEvent::Patch(p) => {
                    let _ = handle.emit(EVENT_PATCH, p);
                }
                StoreEvent::Workspaces => {
                    if let Some(s) = weak.upgrade() {
                        let _ = handle.emit(EVENT_WORKSPACES, s.workspaces());
                    }
                }
                StoreEvent::Note(n) => {
                    let _ = handle.emit(EVENT_NOTE, n);
                }
            });
            let store = self.store.clone();
            let _ = std::thread::Builder::new()
                .name("canvas-note-poll".into())
                .spawn(move || loop {
                    std::thread::sleep(NOTE_POLL);
                    notes::poll_changes(&store);
                });
            crate::commands::canvas::workspace_io::start_backup_scheduler(self.store.clone());
            super::link::ensure_watch();
        }
        self
    }

    pub fn flush(&self) -> Result<(), String> {
        self.store.flush()
    }
}

pub fn flush_on_exit() {
    if let Some(h) = HOST.get() {
        if let Err(e) = h.flush() {
            tracing::warn!("[canvas] flush on exit failed: {e}");
        }
    }
}
