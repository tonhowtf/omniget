//! Embedded terminals: the app side of `omniget_core::core::term`.
//!
//! The sessions, the emulator and the agent state live in the core's
//! [`TermManager`]; this module provides its PTY backend (`portable-pty`,
//! see [`portable`]), turns its notifications into Tauri events, and keeps
//! the Central's `pty_*` contract byte for byte.
//!
//! Sessions live in the app process, so closing the panel or the window
//! never kills them: a remounted view calls [`PtyManager::attach`] and gets
//! the replay-safe history plus a cursor, then keeps reading live output
//! from the `pty://data` event.
//!
//! Events (all payloads carry `id`):
//! - `pty://data` `{id, generation, seq, start, data}`: `data` is base64 of the
//!   raw bytes, `start` their absolute offset in the generation. A view drops
//!   bytes before its cursor and re-attaches on a gap. Ack with `pty_ack(seq)`.
//! - `pty://exit` `{id, code, signal}` (also sent as `term://exit`).
//! - `pty://activity` `{id, running, label, pid}` when the foreground changes.
//! - `pty://stall` `{id}`: the view stopped acking and was detached; attach again.
//! - `term://state` `{id, state, source, since_ms, message, progress}`: agent state.

pub mod portable;

use std::sync::{Arc, OnceLock};

use base64::Engine;
use omniget_core::core::term::agent::AgentSignal;
use omniget_core::core::term::session::foreground::Foreground;
use omniget_core::core::term::session::{self as term_session, TermEvents, TermManager};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub use omniget_core::core::term::session::{history, sanitize};

/// Same shape as before the move (`agent` and `restore` are optional extras).
pub type OpenRequest = term_session::TermOpen;
/// `SessionInfo` gained `agent`, `agent_kind`, `osc_title`, `osc_cwd`.
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

/// The process-wide terminal manager (installed as `core::term` global on
/// first use, so core code reaches it through `session::global()` too).
pub fn term() -> Arc<TermManager> {
    static TERM: OnceLock<Arc<TermManager>> = OnceLock::new();
    TERM.get_or_init(|| {
        term_session::install_global(TermManager::new(Arc::new(portable::PortableBackend)))
    })
    .clone()
}

/// Thin adapter that keeps the Central's `PtyManager` API.
#[derive(Default)]
pub struct PtyManager {
    app: OnceLock<AppHandle>,
}

static MANAGER: OnceLock<PtyManager> = OnceLock::new();

/// The process-wide manager. Needs no setup: the first command hands it the
/// `AppHandle` it emits with.
pub fn manager() -> &'static PtyManager {
    MANAGER.get_or_init(PtyManager::default)
}

impl PtyManager {
    pub fn bind(&self, app: &AppHandle) {
        if self.app.set(app.clone()).is_ok() {
            term().set_events(Arc::new(TauriEvents(app.clone())));
        }
    }

    /// Default history cap for sessions opened from now on.
    pub fn set_history_cap(&self, bytes: usize) {
        term().set_history_cap(bytes);
    }

    pub fn open(&self, req: OpenRequest) -> Result<SessionInfo, String> {
        term().open(req)
    }

    /// Bytes from the Central's xterm.js: keystrokes and its query answers.
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

    /// Kills every session (app exit).
    pub fn shutdown_all(&self) {
        term().shutdown_all();
    }
}
