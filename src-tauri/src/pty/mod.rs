pub mod portable;

use std::sync::{Arc, OnceLock};

use base64::Engine;
use omniget_core::core::term::agent::AgentSignal;
use omniget_core::core::term::session::foreground::Foreground;
use omniget_core::core::term::session::{self as term_session, TermEvents, TermManager};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub use omniget_core::core::term::session::{history, sanitize};

pub type OpenRequest = term_session::TermOpen;
pub type SessionInfo = term_session::TermInfo;
pub type Attached = term_session::Attached;

#[derive(Clone, Serialize)]
struct DataEvent<'a> {
    id: &'a str,
    generation: u64,
    seq: u64,
    start: u64,
    data: String,
}

#[derive(Clone, Serialize)]
struct ExitEvent<'a> {
    id: &'a str,
    code: Option<i32>,
    signal: Option<&'a str>,
}

#[derive(Clone, Serialize)]
struct ActivityEvent<'a> {
    id: &'a str,
    #[serde(flatten)]
    fg: &'a Foreground,
}

#[derive(Clone, Serialize)]
struct IdEvent<'a> {
    id: &'a str,
}

#[derive(Clone, Serialize)]
struct StateEvent<'a> {
    id: &'a str,
    #[serde(flatten)]
    signal: &'a AgentSignal,
}

struct TauriEvents(AppHandle);

impl TermEvents for TauriEvents {
    fn data(&self, id: &str, generation: u64, seq: u64, start: u64, data: &[u8]) {
        let _ = self.0.emit(
            "pty://data",
            DataEvent {
                id,
                generation,
                seq,
                start,
                data: base64::engine::general_purpose::STANDARD.encode(data),
            },
        );
    }

    fn exit(&self, id: &str, code: Option<i32>, signal: Option<&str>) {
        let ev = ExitEvent { id, code, signal };
        let _ = self.0.emit("pty://exit", ev.clone());
        let _ = self.0.emit("term://exit", ev);
    }

    fn activity(&self, id: &str, fg: &Foreground) {
        let _ = self.0.emit("pty://activity", ActivityEvent { id, fg });
    }

    fn stall(&self, id: &str) {
        let _ = self.0.emit("pty://stall", IdEvent { id });
    }

    fn state(&self, id: &str, signal: &AgentSignal) {
        let _ = self.0.emit("term://state", StateEvent { id, signal });
    }
}

pub fn term() -> Arc<TermManager> {
    static TERM: OnceLock<Arc<TermManager>> = OnceLock::new();
    TERM.get_or_init(|| {
        term_session::install_global(TermManager::new(Arc::new(portable::PortableBackend)))
    })
    .clone()
}

#[derive(Default)]
pub struct PtyManager {
    app: OnceLock<AppHandle>,
}

static MANAGER: OnceLock<PtyManager> = OnceLock::new();

pub fn manager() -> &'static PtyManager {
    MANAGER.get_or_init(PtyManager::default)
}

impl PtyManager {
    pub fn bind(&self, app: &AppHandle) {
        if self.app.set(app.clone()).is_ok() {
            term().set_events(Arc::new(TauriEvents(app.clone())));
        }
    }

    pub fn set_history_cap(&self, bytes: usize) {
        term().set_history_cap(bytes);
    }

    pub fn open(&self, req: OpenRequest) -> Result<SessionInfo, String> {
        term().open(req)
    }

    pub fn write(&self, id: &str, data: &str) -> Result<(), String> {
        term().input(id, data.as_bytes())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        term().resize(id, cols, rows)
    }

    pub fn close(&self, id: &str, delete_history: bool) -> Result<(), String> {
        term().close(id, delete_history)
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        term().list()
    }

    pub fn attach(&self, id: &str) -> Result<Attached, String> {
        term().attach(id)
    }

    pub fn detach(&self, id: &str) -> Result<(), String> {
        term().detach(id)
    }

    pub fn ack(&self, id: &str, seq: u64) -> Result<(), String> {
        term().ack(id, seq)
    }

    pub fn clear(&self, id: &str) -> Result<u64, String> {
        term().clear(id)
    }

    pub fn shutdown_all(&self) {
        term().shutdown_all();
    }
}
