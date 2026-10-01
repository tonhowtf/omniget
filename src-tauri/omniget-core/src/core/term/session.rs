//! [`TermManager`]: every terminal of the app (the Central's and the
//! canvas's), heir of the Central's `PtyManager`, with no Tauri dependency.
//!
//! Per session there are two threads:
//! - the **reader** blocks on the PTY and feeds each read to the OSC sniffer,
//!   the emulator (whose answers to terminal queries go straight back to the
//!   PTY, view or no view) and the agent state; then appends to the bounded
//!   history and, while a Central view is attached, to the pending output;
//! - the **pump** coalesces pending output into ~16 ms blocks for the Central
//!   (`TermEvents::data`, flow control of 8 chunks / 64 KiB unacknowledged),
//!   writes the history to disk in batches, refreshes the foreground program,
//!   runs the agent heuristics and flushes an expired synchronized update.
//!   With nothing to do it sleeps on a condvar.
//!
//! The canvas does not read bytes: it subscribes to frames
//! ([`TermManager::subscribe_frames`]). One ticker thread per subscriber
//! wakes on damage and sends `OGT1` frames through a [`FrameSink`] under a
//! credit of two frames in flight.

pub mod foreground;
pub mod history;
pub mod pty;
pub mod sanitize;

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, RwLock, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::agent::state::now_ms;
use super::agent::{AgentKind, AgentSignal, AgentState, AgentTracker, SignalSource};
use super::engine::{self, EngineEvent, ScrollBy, SelectionKind, TermEngine};
use super::input::{self, KeyInput, MouseInput, ScrollInput, SelectInput};
use super::osc::{OscEvent, Sniffer};
use super::screen::{self, ScreenSnapshot};
use super::wire::{self, SlotInfo, SlotInput, Visible, WireSource, WireSubscriber};
use foreground::{Foreground, NameCache};
use history::History;
use pty::{PtyBackend, PtyProcess, PtyWaiter};

/// Coalescing window for Central output events.
const COALESCE: Duration = Duration::from_millis(16);
/// Central flow-control window: unacknowledged chunks and bytes.
const WINDOW_CHUNKS: usize = 8;
const WINDOW_BYTES: usize = 64 * 1024;
/// Pending output past this blocks the reader (backpressure on the program).
const PENDING_MAX: usize = 1024 * 1024;
/// A full window with no ack for this long detaches the Central view.
const STALL: Duration = Duration::from_secs(5);
/// History reaches the disk at most this often.
const DISK_EVERY: Duration = Duration::from_millis(500);
/// Foreground label refresh after output, at most this often.
#[cfg(unix)]
const FG_EVERY: Duration = Duration::from_millis(400);
#[cfg(not(unix))]
const FG_EVERY: Duration = Duration::from_secs(3);
/// Agent heuristics cadence while there is something to decide.
const AGENT_EVERY: Duration = Duration::from_millis(500);
/// Upper bound for sessions alive at once.
const MAX_SESSIONS: usize = 64;
/// Scrollback lines kept by the emulator.
const SCROLLBACK: usize = 10_000;
/// `user_typing` window.
const TYPING_WINDOW: Duration = Duration::from_millis(1500);
/// Frame pacing of a subscriber (~60 fps at most).
const FRAME_MIN: Duration = Duration::from_millis(16);
/// Idle wait of a ticker (nothing dirty).
const TICK_IDLE: Duration = Duration::from_millis(250);

/// Played into the emulator (never into the saved transcript) after a
/// replayed transcript: colours, alternate screen, input modes and cursor
/// back to their defaults. Kept out of the transcript because the replay
/// sanitizer reads an unmatched alternate-screen exit as "drop the past".
const REPLAY_RESET: &[u8] = b"\x1b[0m\x1b[?1049l\x1b[?2004l\x1b[?1l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\x1b[?1005l\x1b[?1006l\x1b[?1015l\x1b>\x1b[?25h";

/// Screen generations are unique across sessions, so a subscriber sees a new
/// process under a reused id (restart, role swap, reload) as a new screen and
/// sends it whole instead of diffing against the old one.
static NEXT_SCREEN_GEN: AtomicU32 = AtomicU32::new(1);

fn next_screen_gen() -> u32 {
    NEXT_SCREEN_GEN.fetch_add(1, Ordering::Relaxed).max(1)
}

fn err(code: &str, msg: impl std::fmt::Display) -> String {
    format!("{code}: {msg}")
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ------------------------------------------------------------------ types

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TermOpen {
    /// Chosen by the caller (stable across remounts); generated when absent.
    pub id: Option<String>,
    pub cwd: Option<String>,
    /// argv. Empty or absent opens the user's default shell (login shell on
    /// unix, `%ComSpec%` on Windows). On unix a command runs through the login
    /// shell so it sees the user's PATH.
    pub command: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub title: Option<String>,
    /// Typed into the terminal after start, not submitted.
    #[serde(alias = "initialInput")]
    pub initial_input: Option<String>,
    /// History cap in bytes (default 2 MiB, minimum 64 KiB).
    #[serde(alias = "historyCap")]
    pub history_cap: Option<usize>,
    /// What runs in it, for the state heuristics (default: from argv[0]).
    pub agent: Option<AgentKind>,
    /// When the program is a path that no longer exists (a shell that was
    /// uninstalled, a moved binary), open an available shell instead and say
    /// so on screen. Default on (Settings → Canvas → Terminal).
    #[serde(alias = "shellFallback")]
    pub shell_fallback: Option<bool>,
    /// Replay the sanitized transcript of a previous run of this id into the
    /// new screen (the canvas reopening a node after a restart).
    pub restore: Option<bool>,
    /// Inherited variables to drop from the child's environment (the canvas
    /// strips the agent session variables of whoever launched the app).
    #[serde(alias = "envRemove")]
    pub env_remove: Option<Vec<String>>,
}

/// Variables of an agent session the app itself may be running inside
/// (started from a Claude Code or Codex terminal). A child agent that
/// inherits them believes it is nested (`CLAUDE_CODE_CHILD_SESSION` turns
/// transcript saving off, `CODEX_SANDBOX` changes Codex's sandbox), so the
/// canvas removes them. `CODEX_HOME` is user configuration and stays.
pub fn inherited_agent_session_vars() -> Vec<String> {
    inherited_agent_session_vars_from(std::env::vars_os().filter_map(|(k, _)| k.into_string().ok()))
}

fn inherited_agent_session_vars_from(keys: impl Iterator<Item = String>) -> Vec<String> {
    keys.filter(|k| {
        k == "CLAUDECODE"
            || k.starts_with("CLAUDE_CODE_")
            || k == "CLAUDE_AGENT_SDK_VERSION"
            || k == "CLAUDE_PID"
            || k == "CLAUDE_EFFORT"
            || (k.starts_with("CODEX_") && k != "CODEX_HOME")
            || k == "GEMINI_CLI"
    })
    .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct TermInfo {
    pub id: String,
    pub title: String,
    pub cwd: Option<String>,
    pub command: Vec<String>,
    pub pid: Option<u32>,
    pub cols: u16,
    pub rows: u16,
    pub alive: bool,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<String>,
    pub created_at: i64,
    pub foreground: Foreground,
    pub agent: AgentSignal,
    pub agent_kind: AgentKind,
    /// Last title the program set (OSC 0/2).
    pub osc_title: Option<String>,
    /// Last cwd the shell reported (OSC 7).
    pub osc_cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Attached {
    pub info: Option<TermInfo>,
    /// base64 of the sanitized history.
    pub history: String,
    pub generation: u64,
    /// Absolute offset the history ends at: live bytes before it are duplicates.
    pub offset: u64,
    /// false when the session is gone and only its transcript remains.
    pub alive: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SearchHit {
    pub start_col: u16,
    pub start_row: u16,
    pub end_col: u16,
    pub end_row: u16,
}

/// Where frames go (the app: a Tauri `Channel`; tests: a `Vec`).
pub trait FrameSink: Send + Sync {
    /// Returns false when the receiver is gone (the subscription is dropped).
    fn send(&self, frame: Vec<u8>) -> bool;
}

/// Notifications out of the manager. All default to no-ops.
pub trait TermEvents: Send + Sync {
    /// Central byte stream (`pty://data`).
    fn data(&self, _id: &str, _generation: u64, _seq: u64, _start: u64, _data: &[u8]) {}
    fn exit(&self, _id: &str, _code: Option<i32>, _signal: Option<&str>) {}
    /// Foreground program changed (`pty://activity`).
    fn activity(&self, _id: &str, _fg: &Foreground) {}
    /// A Central view stopped acknowledging and was detached (`pty://stall`).
    fn stall(&self, _id: &str) {}
    /// Agent state changed (`term://state`).
    fn state(&self, _id: &str, _signal: &AgentSignal) {}
    /// OSC 52 copy request from the program.
    fn clipboard(&self, _id: &str, _text: &str) {}
}

pub type ByteObserver = Arc<dyn Fn(&[u8]) + Send + Sync>;

// ---------------------------------------------------------------- session

struct Out {
    generation: u64,
    offset: u64,
    history: History,
    viewers: u32,
    pending: Vec<u8>,
    pending_start: u64,
    pending_since: Option<Instant>,
    next_seq: u64,
    inflight: VecDeque<(u64, usize)>,
    inflight_bytes: usize,
    last_ack: Instant,
    disk_due: Option<Instant>,
    fg_due: Option<Instant>,
    fg_last_run: Option<Instant>,
    agent_due: Option<Instant>,
    sync_due: Option<Instant>,
    foreground: Foreground,
    eof: bool,
    closed: bool,
    exit_code: Option<i32>,
    exit_signal: Option<String>,
}

impl Out {
    fn window_full(&self) -> bool {
        self.inflight.len() >= WINDOW_CHUNKS || self.inflight_bytes >= WINDOW_BYTES
    }

    fn reset_flow(&mut self) {
        self.pending.clear();
        self.pending_start = self.offset;
        self.pending_since = None;
        self.inflight.clear();
        self.inflight_bytes = 0;
        self.last_ack = Instant::now();
    }
}

/// Emulator side of a session.
struct TermState {
    engine: Box<dyn TermEngine>,
    sniffer: Sniffer,
    agent: AgentTracker,
    content_seq: u64,
    generation: u32,
    osc_title: Option<String>,
    osc_cwd: Option<String>,
    changed_ms: i64,
    alive: bool,
    /// ⌘⇧B: new output does not move the view.
    scroll_lock: bool,
}

impl TermState {
    fn touch(&mut self) {
        self.content_seq += 1;
        self.changed_ms = now_ms();
    }
}

struct Session {
    id: String,
    title: String,
    cwd: Option<String>,
    command: Vec<String>,
    shell_label: String,
    pid: Option<u32>,
    created_at: i64,
    size: Mutex<(u16, u16)>,
    process: Mutex<Box<dyn PtyProcess>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    out: Mutex<Out>,
    cv: Condvar,
    names: Mutex<NameCache>,
    term: Mutex<TermState>,
    /// Central views attached (mirror of `Out::viewers`): while one is, its
    /// xterm.js answers terminal queries and the engine must not answer too.
    central_viewers: AtomicU32,
    observers: Mutex<Vec<(u64, ByteObserver)>>,
    last_user_input: Mutex<Option<Instant>>,
}

impl Session {
    fn out(&self) -> MutexGuard<'_, Out> {
        lock(&self.out)
    }

    fn term(&self) -> MutexGuard<'_, TermState> {
        lock(&self.term)
    }

    fn info(&self) -> TermInfo {
        let (cols, rows) = *lock(&self.size);
        let (alive, exit_code, exit_signal, foreground) = {
            let o = self.out();
            (
                !o.eof,
                o.exit_code,
                o.exit_signal.clone(),
                o.foreground.clone(),
            )
        };
        let (agent, agent_kind, osc_title, osc_cwd) = {
            let t = self.term();
            (
                t.agent.signal().clone(),
                t.agent.kind(),
                t.osc_title.clone(),
                t.osc_cwd.clone(),
            )
        };
        TermInfo {
            id: self.id.clone(),
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            command: self.command.clone(),
            pid: self.pid,
            cols,
            rows,
            alive,
            exit_code,
            exit_signal,
            created_at: self.created_at,
            foreground,
            agent,
            agent_kind,
            osc_title,
            osc_cwd,
        }
    }

    /// Runs the state heuristics soon even if the program prints nothing
    /// (an answer that leaves a silent program behind).
    fn wake_heuristics(&self) {
        let mut o = self.out();
        if !o.eof && !o.closed {
            let at = Instant::now() + AGENT_EVERY;
            o.agent_due = Some(o.agent_due.map_or(at, |d| d.min(at)));
            o.fg_due.get_or_insert(at);
        }
        drop(o);
        self.cv.notify_all();
    }

    fn compute_foreground(&self) -> Foreground {
        let pgid = lock(&self.process).foreground_pgid();
        let mut names = lock(&self.names);
        foreground::resolve(pgid, self.pid, &self.shell_label, &mut names)
    }

    fn write_raw(&self, data: &[u8]) -> Result<(), String> {
        let mut w = lock(&self.writer);
        let Some(w) = w.as_mut() else {
            return Err(err("PTY_CLOSED", "terminal is closed"));
        };
        w.write_all(data)
            .and_then(|_| w.flush())
            .map_err(|e| err("PTY_WRITE", e))
    }
}

/// State shared with the session threads and tickers.
struct Shared {
    events: RwLock<Option<Arc<dyn TermEvents>>>,
    /// Damage counter + condvar the tickers sleep on.
    bus: Mutex<u64>,
    bus_cv: Condvar,
}

impl Shared {
    fn events(&self) -> Option<Arc<dyn TermEvents>> {
        self.events
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn damage(&self) {
        *lock(&self.bus) += 1;
        self.bus_cv.notify_all();
    }

    fn state(&self, id: &str, sig: &AgentSignal) {
        if let Some(ev) = self.events() {
            ev.state(id, sig);
        }
        self.damage();
    }

    /// Waits until the damage counter moves past `seen` or `timeout` ends.
    fn wait(&self, seen: u64, timeout: Duration) -> u64 {
        let g = lock(&self.bus);
        if *g != seen {
            return *g;
        }
        let (g, _) = self
            .bus_cv
            .wait_timeout(g, timeout)
            .unwrap_or_else(|e| e.into_inner());
        *g
    }
}

struct FrameSub {
    sink: Arc<dyn FrameSink>,
    wire: Mutex<WireSubscriber>,
    stop: AtomicBool,
}

// ---------------------------------------------------------------- manager

pub struct TermManager {
    backend: Arc<dyn PtyBackend>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    shared: Arc<Shared>,
    default_cap: Mutex<Option<usize>>,
    subs: Mutex<HashMap<String, Arc<FrameSub>>>,
    me: OnceLock<Weak<TermManager>>,
}

static GLOBAL: OnceLock<Arc<TermManager>> = OnceLock::new();

/// The process-wide manager, once the app installed it.
pub fn global() -> Option<Arc<TermManager>> {
    GLOBAL.get().cloned()
}

/// Installs the process-wide manager (first call wins) and returns it.
pub fn install_global(m: Arc<TermManager>) -> Arc<TermManager> {
    GLOBAL.get_or_init(|| m).clone()
}

impl WireSource for TermManager {
    fn visit(&self, id: &str, f: &mut dyn FnMut(&SlotInput<'_>)) -> bool {
        let Some(s) = lock(&self.sessions).get(id).cloned() else {
            return false;
        };
        let t = s.term();
        let title = t
            .osc_title
            .clone()
            .or_else(|| t.engine.title())
            .unwrap_or_else(|| s.title.clone());
        let cwd = t
            .osc_cwd
            .clone()
            .or_else(|| s.cwd.clone())
            .unwrap_or_default();
        f(&SlotInput {
            engine: &*t.engine,
            content_seq: t.content_seq,
            generation: t.generation,
            agent: t.agent.signal(),
            agent_seq: t.agent.seq(),
            title: &title,
            cwd: &cwd,
            alive: t.alive,
        });
        true
    }
}

impl TermManager {
    pub fn new(backend: Arc<dyn PtyBackend>) -> Arc<Self> {
        let m = Arc::new(Self {
            backend,
            sessions: Mutex::new(HashMap::new()),
            shared: Arc::new(Shared {
                events: RwLock::new(None),
                bus: Mutex::new(0),
                bus_cv: Condvar::new(),
            }),
            default_cap: Mutex::new(None),
            subs: Mutex::new(HashMap::new()),
            me: OnceLock::new(),
        });
        let _ = m.me.set(Arc::downgrade(&m));
        m
    }

    pub fn set_events(&self, events: Arc<dyn TermEvents>) {
        *self
            .shared
            .events
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(events);
    }

    /// Default history cap for sessions opened from now on.
    pub fn set_history_cap(&self, bytes: usize) {
        *lock(&self.default_cap) = Some(bytes.max(history::MIN_CAP));
    }

    fn get(&self, id: &str) -> Result<Arc<Session>, String> {
        lock(&self.sessions)
            .get(id)
            .cloned()
            .ok_or_else(|| err("PTY_NOT_FOUND", format!("no terminal {id}")))
    }

    pub fn open(&self, req: TermOpen) -> Result<TermInfo, String> {
        let id = match req.id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) => {
                if history::path_for(id).is_none() && history::dir().is_some() {
                    return Err(err(
                        "PTY_BAD_ID",
                        "id must be [A-Za-z0-9._-], up to 128 chars",
                    ));
                }
                id.to_string()
            }
            None => uuid::Uuid::new_v4().to_string(),
        };
        {
            let map = lock(&self.sessions);
            if let Some(existing) = map.get(&id) {
                // Opening an id that is alive is an attach in disguise.
                return Ok(existing.info());
            }
            if map.len() >= MAX_SESSIONS {
                return Err(err(
                    "PTY_LIMIT",
                    format!("at most {MAX_SESSIONS} terminals"),
                ));
            }
        }

        let cols = req.cols.filter(|c| *c > 0).unwrap_or(80);
        let rows = req.rows.filter(|r| *r > 0).unwrap_or(24);
        let mut argv: Vec<String> = req
            .command
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        let mut notice: Option<String> = None;
        if req.shell_fallback.unwrap_or(true) && pty::missing_program(&argv) {
            notice = Some(format!(
                "\x1b[33m[OmniGet] {} was not found; opened {} instead.\x1b[0m\r\n",
                argv[0],
                pty::available_shell()
            ));
            argv.clear();
        }
        let (mut spec, shell_label) = pty::spec_for(&argv);
        let cwd = req
            .cwd
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .or_else(dirs::home_dir);
        spec.cwd = cwd.clone();
        spec.cols = cols;
        spec.rows = rows;
        spec.env.push(("TERM".into(), "xterm-256color".into()));
        spec.env.push(("COLORTERM".into(), "truecolor".into()));
        spec.env.push(("TERM_PROGRAM".into(), "OmniGet".into()));
        spec.env.push((
            "TERM_PROGRAM_VERSION".into(),
            env!("CARGO_PKG_VERSION").into(),
        ));
        #[cfg(target_os = "macos")]
        if std::env::var_os("LANG").is_none() {
            spec.env.push(("LANG".into(), "en_US.UTF-8".into()));
        }
        spec.env.push(("OMNIGET_TERMINAL_ID".into(), id.clone()));
        let added = req.env.clone().unwrap_or_default();
        for k in req.env_remove.clone().unwrap_or_default() {
            if !k.is_empty() && !added.contains_key(&k) {
                spec.env_remove.push(k);
            }
        }
        for (k, v) in added {
            if !k.is_empty() && !k.contains('=') {
                spec.env.push((k, v));
            }
        }

        let backend = self.backend.clone();
        let mut process = crate::core::process::spawn_retrying_busy(|| {
            backend.spawn(&spec).map_err(std::io::Error::other)
        })
        .map_err(|e| err("PTY_SPAWN", e))?;
        let reader = process.take_reader().map_err(|e| err("PTY_OPEN", e))?;
        let writer = process.take_writer().map_err(|e| err("PTY_OPEN", e))?;
        let waiter = process.take_waiter();
        let pid = process.pid();

        let cap = req
            .history_cap
            .or(*lock(&self.default_cap))
            .unwrap_or(history::DEFAULT_CAP);
        let log_path = history::path_for(&id);
        if let Some(p) = &log_path {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
        }
        let title = req
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| shell_label.clone());

        let mut engine = engine::new_engine(cols, rows, SCROLLBACK);
        let mut hist = History::new(cap, log_path);
        if req.restore.unwrap_or(false) {
            if let Some(raw) = history::read_tail(&id, cap) {
                let mut replay = sanitize::sanitize_for_replay(&raw);
                // The replayed past belongs to a process that is gone: the
                // new one starts with no input mode of its own (a leftover
                // bracketed paste would wrap our pastes in markers the new
                // program never asked for) and on the main screen.
                replay.extend_from_slice(b"\x1b[0m\r\n");
                engine.advance(&replay);
                engine.advance(REPLAY_RESET);
                // Answers to replayed queries must never reach the new program.
                let _ = engine.take_events();
                hist.push(&replay);
            }
        }
        if let Some(n) = &notice {
            engine.advance(n.as_bytes());
            let _ = engine.take_events();
            hist.push(n.as_bytes());
        }
        let kind = req
            .agent
            .filter(|k| *k != AgentKind::Unknown && notice.is_none())
            .unwrap_or_else(|| {
                argv.first()
                    .map(|a| AgentKind::from_label(&foreground::label_for(a)))
                    .unwrap_or(AgentKind::Shell)
            });

        let session = Arc::new(Session {
            id: id.clone(),
            title,
            cwd: cwd.map(|p| p.display().to_string()),
            command: argv,
            shell_label: shell_label.clone(),
            pid,
            created_at: now_ms(),
            size: Mutex::new((cols, rows)),
            process: Mutex::new(process),
            writer: Mutex::new(Some(writer)),
            out: Mutex::new(Out {
                generation: 1,
                offset: 0,
                history: hist,
                viewers: 0,
                pending: Vec::new(),
                pending_start: 0,
                pending_since: None,
                next_seq: 1,
                inflight: VecDeque::new(),
                inflight_bytes: 0,
                last_ack: Instant::now(),
                disk_due: None,
                fg_due: None,
                fg_last_run: None,
                agent_due: None,
                sync_due: None,
                foreground: Foreground {
                    running: false,
                    label: shell_label,
                    pid,
                },
                eof: false,
                closed: false,
                exit_code: None,
                exit_signal: None,
            }),
            cv: Condvar::new(),
            names: Mutex::new(NameCache::default()),
            term: Mutex::new(TermState {
                engine,
                sniffer: Sniffer::new(),
                agent: AgentTracker::new(kind),
                content_seq: 1,
                generation: next_screen_gen(),
                osc_title: None,
                osc_cwd: None,
                changed_ms: now_ms(),
                alive: true,
                scroll_lock: false,
            }),
            central_viewers: AtomicU32::new(0),
            observers: Mutex::new(Vec::new()),
            last_user_input: Mutex::new(None),
        });

        lock(&self.sessions).insert(id.clone(), session.clone());

        {
            let s = session.clone();
            let sh = self.shared.clone();
            std::thread::Builder::new()
                .name(format!("term-read-{id}"))
                .spawn(move || read_loop(s, sh, reader, waiter))
                .map_err(|e| err("PTY_THREAD", e))?;
        }
        {
            let s = session.clone();
            let sh = self.shared.clone();
            std::thread::Builder::new()
                .name(format!("term-pump-{id}"))
                .spawn(move || pump_loop(s, sh))
                .map_err(|e| err("PTY_THREAD", e))?;
        }

        if let Some(text) = req.initial_input.filter(|t| !t.is_empty()) {
            // Give the shell a moment to print its prompt first.
            let s = session.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                let _ = s.write_raw(text.as_bytes());
            });
        }
        self.shared.damage();
        Ok(session.info())
    }

    /// Raw bytes to the program (not counted as the user typing).
    pub fn write(&self, id: &str, bytes: &[u8]) -> Result<(), String> {
        let s = self.get(id)?;
        // An Enter written for someone (a remote answer, a raw cord) still
        // answers a question on screen and starts a turn.
        if bytes.contains(&b'\r') {
            let changed = {
                let mut t = s.term();
                t.agent
                    .on_user_input(true, Instant::now())
                    .then(|| t.agent.signal().clone())
            };
            if let Some(sig) = changed {
                self.shared.state(&s.id, &sig);
            }
            s.wake_heuristics();
        }
        s.write_raw(bytes)
    }

    /// Keystrokes from the user: marks `user_typing`, snaps a scrolled-back
    /// view to the bottom, and lets a submit answer a sticky agent state.
    pub fn input(&self, id: &str, bytes: &[u8]) -> Result<(), String> {
        let s = self.get(id)?;
        *lock(&s.last_user_input) = Some(Instant::now());
        let submit = bytes.contains(&b'\r');
        let changed = {
            let mut t = s.term();
            if t.engine.display_offset() > 0 {
                t.engine.scroll_display(ScrollBy::Bottom);
                t.touch();
            }
            t.agent
                .on_user_input(submit, Instant::now())
                .then(|| t.agent.signal().clone())
        };
        if let Some(sig) = changed {
            self.shared.state(&s.id, &sig);
        }
        if submit {
            s.wake_heuristics();
        }
        s.write_raw(bytes)
    }

    fn modes(&self, s: &Session) -> engine::Modes {
        s.term().engine.modes()
    }

    pub fn paste(&self, id: &str, text: &str) -> Result<(), String> {
        let s = self.get(id)?;
        let bytes = input::encode_paste(text, self.modes(&s).bracketed_paste);
        *lock(&s.last_user_input) = Some(Instant::now());
        s.write_raw(&bytes)
    }

    /// Enter, sent by another agent or a cord (not the user).
    pub fn submit(&self, id: &str) -> Result<(), String> {
        let s = self.get(id)?;
        let changed = {
            let mut t = s.term();
            t.agent
                .on_user_input(true, Instant::now())
                .then(|| t.agent.signal().clone())
        };
        if let Some(sig) = changed {
            self.shared.state(&s.id, &sig);
        }
        s.write_raw(b"\r")
    }

    /// Delivers a prompt the way the Maestri does: bracketed paste, a short
    /// pause (TUIs debounce paste), then Enter. Returns right away.
    pub fn send_prompt(&self, id: &str, text: &str) -> Result<(), String> {
        let s = self.get(id)?;
        let bytes = input::encode_paste(text, self.modes(&s).bracketed_paste);
        s.write_raw(&bytes)?;
        let me = self.me.get().and_then(Weak::upgrade);
        let id = id.to_string();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            if let Some(m) = me {
                let _ = m.submit(&id);
            }
        });
        Ok(())
    }

    pub fn key(&self, id: &str, key: &KeyInput) -> Result<(), String> {
        let s = self.get(id)?;
        match input::encode_key(key, &self.modes(&s)) {
            Some(bytes) => self.input(id, &bytes),
            None => Ok(()),
        }
    }

    pub fn text(&self, id: &str, text: &str) -> Result<(), String> {
        if text.is_empty() {
            return Ok(());
        }
        self.input(id, text.as_bytes())
    }

    /// Returns true when the program consumed the event (mouse reporting on,
    /// or the wheel became arrows/scrollback); false = the front does local
    /// selection.
    pub fn mouse(&self, id: &str, m: &MouseInput) -> Result<bool, String> {
        let s = self.get(id)?;
        let modes = self.modes(&s);
        if m.mods & input::MOD_SHIFT == 0 {
            if let Some(bytes) = input::encode_mouse(m, &modes) {
                s.write_raw(&bytes)?;
                return Ok(true);
            }
        }
        if m.kind == "wheel" {
            self.scroll(
                id,
                &ScrollInput {
                    lines: m.wheel,
                    ..Default::default()
                },
            )?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn scroll(&self, id: &str, sc: &ScrollInput) -> Result<(), String> {
        let s = self.get(id)?;
        let modes = self.modes(&s);
        if sc.to.is_none() && sc.pages == 0 {
            if let Some(bytes) = input::encode_alternate_scroll(sc.lines, &modes) {
                return s.write_raw(&bytes);
            }
        }
        let by = match sc.to.as_deref() {
            Some("top") => ScrollBy::Top,
            Some("bottom") => ScrollBy::Bottom,
            _ if sc.pages != 0 => ScrollBy::Pages(sc.pages),
            _ => ScrollBy::Lines(sc.lines),
        };
        let mut t = s.term();
        t.engine.scroll_display(by);
        t.touch();
        drop(t);
        self.shared.damage();
        Ok(())
    }

    pub fn select(&self, id: &str, sel: &SelectInput) -> Result<(), String> {
        let s = self.get(id)?;
        let right = sel.side.as_deref() == Some("right");
        let kind = match sel.ty.as_deref() {
            Some("block") => SelectionKind::Block,
            Some("semantic") => SelectionKind::Semantic,
            Some("lines") => SelectionKind::Lines,
            _ => SelectionKind::Simple,
        };
        let mut t = s.term();
        match sel.kind.as_str() {
            "start" => t.engine.selection_start(kind, sel.col, sel.row, right),
            "update" => t.engine.selection_update(sel.col, sel.row, right),
            "all" => t.engine.selection_all(),
            _ => t.engine.selection_clear(),
        }
        t.touch();
        drop(t);
        self.shared.damage();
        Ok(())
    }

    pub fn copy(&self, id: &str) -> Result<Option<String>, String> {
        Ok(self.get(id)?.term().engine.selection_text())
    }

    pub fn search(
        &self,
        id: &str,
        query: &str,
        backwards: bool,
    ) -> Result<Option<SearchHit>, String> {
        let s = self.get(id)?;
        let mut t = s.term();
        // The find bar searches for the text as typed, not a regex.
        let hit = t.engine.search(&literal_regex(query), backwards);
        t.touch();
        drop(t);
        self.shared.damage();
        Ok(hit.map(|h| SearchHit {
            start_col: h.start.0,
            start_row: h.start.1,
            end_col: h.end.0,
            end_row: h.end.1,
        }))
    }

    /// Focus changed on the node: focus report (1004) and `success` → `idle`.
    pub fn focus(&self, id: &str, focused: bool) -> Result<(), String> {
        let s = self.get(id)?;
        let (bytes, changed) = {
            let mut t = s.term();
            let bytes = input::encode_focus(focused, &t.engine.modes());
            let changed =
                (focused && t.agent.acknowledge(Instant::now())).then(|| t.agent.signal().clone());
            (bytes, changed)
        };
        if let Some(sig) = changed {
            self.shared.state(&s.id, &sig);
        }
        if let Some(b) = bytes {
            let _ = s.write_raw(&b);
        }
        Ok(())
    }

    pub fn link_uri(&self, id: &str, link_id: u32) -> Option<String> {
        self.get(id).ok()?.term().engine.link_uri(link_id)
    }

    /// "The last one wins": every call applies, whoever sent it.
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        if cols == 0 || rows == 0 {
            return Err(err("PTY_BAD_SIZE", "cols and rows must be positive"));
        }
        let s = self.get(id)?;
        let mut size = lock(&s.size);
        if *size == (cols, rows) {
            return Ok(());
        }
        lock(&s.process)
            .resize(cols, rows)
            .map_err(|e| err("PTY_RESIZE", e))?;
        *size = (cols, rows);
        drop(size);
        let mut t = s.term();
        t.engine.resize(cols, rows);
        t.touch();
        drop(t);
        self.shared.damage();
        Ok(())
    }

    /// Kills the program (its whole process group / job) and forgets the
    /// session. The transcript stays on disk unless `delete_history`.
    pub fn close(&self, id: &str, delete_history: bool) -> Result<(), String> {
        let removed = lock(&self.sessions).remove(id);
        if let Some(s) = removed {
            lock(&s.process).kill();
            // Dropping the writer sends EOF for programs that ignore the kill.
            lock(&s.writer).take();
            let mut o = s.out();
            o.closed = true;
            if delete_history {
                o.history = History::new(history::MIN_CAP, None);
            }
            drop(o);
            s.cv.notify_all();
            lock(&s.observers).clear();
        }
        if delete_history {
            history::forget_agent_session(id);
            if let Some(p) = history::path_for(id) {
                // The pump may still flush once; give it the chance, then remove.
                let _ = std::fs::remove_file(&p);
                std::thread::spawn(move || {
                    std::thread::sleep(DISK_EVERY * 2);
                    let _ = std::fs::remove_file(&p);
                });
            }
        }
        self.shared.damage();
        Ok(())
    }

    pub fn list(&self) -> Vec<TermInfo> {
        let sessions: Vec<Arc<Session>> = lock(&self.sessions).values().cloned().collect();
        let mut out: Vec<TermInfo> = sessions.iter().map(|s| s.info()).collect();
        out.sort_by_key(|i| i.created_at);
        out
    }

    pub fn info(&self, id: &str) -> Option<TermInfo> {
        self.get(id).ok().map(|s| s.info())
    }

    pub fn screen_text(&self, id: &str, last_lines: usize) -> Result<ScreenSnapshot, String> {
        let s = self.get(id)?;
        let t = s.term();
        Ok(screen::snapshot(&*t.engine, last_lines, t.changed_ms))
    }

    /// Screen text of a terminal with no live session in this run (unloaded,
    /// or not started since the app launched): its transcript on disk played
    /// into a scratch emulator of `cols`×`rows`.
    pub fn transcript_text(
        &self,
        id: &str,
        last_lines: usize,
        cols: u16,
        rows: u16,
    ) -> Option<ScreenSnapshot> {
        let cap = lock(&self.default_cap).unwrap_or(history::DEFAULT_CAP);
        let raw = history::read_tail(id, cap)?;
        let changed = history::path_for(id)
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let mut engine = engine::new_engine(cols.max(20), rows.max(5), SCROLLBACK);
        let mut replay = sanitize::sanitize_for_replay(&raw);
        replay.extend_from_slice(b"\x1b[0m\x1b[?1049l");
        engine.advance(&replay);
        let _ = engine.take_events();
        Some(screen::snapshot(&*engine, last_lines, changed))
    }

    /// Whether the program in terminal `id` is busy right now: a busy agent
    /// state, or (for a shell) a program other than the shell in the
    /// foreground, even a silent one (`sleep 60`). `None` when there is no
    /// live session. Routines use it for "skip if busy".
    pub fn is_busy(&self, id: &str) -> Option<bool> {
        let s = self.get(id).ok()?;
        let fg_busy = {
            let o = s.out();
            if o.eof || o.closed {
                return Some(false);
            }
            o.foreground.running && !AgentKind::from_label(&o.foreground.label).is_agent()
        };
        // Fresh look at the foreground group: the cached one can be up to a
        // few hundred ms old.
        let fg_busy = fg_busy || {
            let fg = s.compute_foreground();
            fg.running && !AgentKind::from_label(&fg.label).is_agent()
        };
        let state = s.term().agent.signal().state;
        Some(
            fg_busy
                || matches!(
                    state,
                    AgentState::Thinking
                        | AgentState::Working
                        | AgentState::Compacting
                        | AgentState::Looping
                        | AgentState::WaitingPeer
                ),
        )
    }

    /// What runs in terminal `id`, as the state tracker sees it.
    pub fn agent_kind(&self, id: &str) -> Option<AgentKind> {
        self.get(id).ok().map(|s| s.term().agent.kind())
    }

    pub fn agent_state(&self, id: &str) -> Option<AgentSignal> {
        self.get(id).ok().map(|s| s.term().agent.signal().clone())
    }

    /// Explicit state from the app (a cord waiting on a peer, the bridge).
    pub fn set_agent_state(
        &self,
        id: &str,
        state: AgentState,
        source: SignalSource,
        message: Option<String>,
    ) -> Result<(), String> {
        let s = self.get(id)?;
        let changed = {
            let mut t = s.term();
            t.agent
                .propose(state, source, message, Instant::now())
                .then(|| t.agent.signal().clone())
        };
        if let Some(sig) = changed {
            self.shared.state(&s.id, &sig);
        }
        Ok(())
    }

    /// A hook event (Claude-shaped JSON) for the terminal `id`, routed by
    /// `OMNIGET_TERMINAL_ID`.
    pub fn hook(&self, id: &str, event: &str, payload: &serde_json::Value) -> Result<(), String> {
        let s = self.get(id)?;
        // Remember the agent's own session, so a relaunch can resume it.
        if let Some(sid) = history::session_id_of(payload) {
            history::save_agent_session(id, &sid);
        }
        let changed = {
            let mut t = s.term();
            t.agent
                .on_hook(event, payload, Instant::now())
                .then(|| t.agent.signal().clone())
        };
        if let Some(sig) = changed {
            self.shared.state(&s.id, &sig);
        }
        Ok(())
    }

    /// The user typed in this terminal in the last 1.5 s (an agent should not
    /// paste into it now).
    pub fn user_typing(&self, id: &str) -> bool {
        self.get(id)
            .ok()
            .and_then(|s| *lock(&s.last_user_input))
            .is_some_and(|t| t.elapsed() < TYPING_WINDOW)
    }

    // -------------------------------------------------- frame subscribers

    /// Starts (or replaces) the frame subscription `key` (the webview label).
    pub fn subscribe_frames(
        &self,
        key: &str,
        sink: Arc<dyn FrameSink>,
        visible: Vec<Visible>,
    ) -> Vec<SlotInfo> {
        self.unsubscribe_frames(key);
        let mut wire = WireSubscriber::new();
        let slots = wire.set_visible(&visible);
        let sub = Arc::new(FrameSub {
            sink,
            wire: Mutex::new(wire),
            stop: AtomicBool::new(false),
        });
        lock(&self.subs).insert(key.to_string(), sub.clone());
        let me = self.me.get().cloned().unwrap_or_default();
        let shared = self.shared.clone();
        let key_owned = key.to_string();
        let _ = std::thread::Builder::new()
            .name(format!("term-tick-{key}"))
            .spawn(move || ticker(me, shared, sub, key_owned));
        self.shared.damage();
        slots
    }

    pub fn set_visible(&self, key: &str, visible: Vec<Visible>) -> Vec<SlotInfo> {
        let Some(sub) = lock(&self.subs).get(key).cloned() else {
            return Vec::new();
        };
        let slots = lock(&sub.wire).set_visible(&visible);
        self.shared.damage();
        slots
    }

    pub fn frame_done(&self, key: &str, seq: u32) {
        if let Some(sub) = lock(&self.subs).get(key).cloned() {
            lock(&sub.wire).frame_done(seq);
            self.shared.damage();
        }
    }

    pub fn unsubscribe_frames(&self, key: &str) {
        if let Some(old) = lock(&self.subs).remove(key) {
            old.stop.store(true, Ordering::Relaxed);
            self.shared.damage();
        }
    }

    // ---------------------------------------------- Central byte viewers

    /// Two-step attach: returns the sanitized history and the cursor the live
    /// stream continues from, and registers a view so `data` flows. For an id
    /// with no live session, returns its transcript from disk.
    pub fn attach(&self, id: &str) -> Result<Attached, String> {
        let s = match self.get(id) {
            Ok(s) => s,
            Err(e) => {
                let cap = lock(&self.default_cap).unwrap_or(history::DEFAULT_CAP);
                return match history::read_tail(id, cap) {
                    Some(raw) => Ok(Attached {
                        info: None,
                        history: b64(&sanitize::sanitize_for_replay(&raw)),
                        generation: 0,
                        offset: 0,
                        alive: false,
                    }),
                    None => Err(e),
                };
            }
        };
        let (raw, generation, offset) = {
            let mut o = s.out();
            if o.viewers == 0 {
                o.reset_flow();
            }
            o.viewers += 1;
            s.central_viewers.store(o.viewers, Ordering::Relaxed);
            (o.history.bytes().to_vec(), o.generation, o.offset)
        };
        s.cv.notify_all();
        Ok(Attached {
            info: Some(s.info()),
            history: b64(&sanitize::sanitize_for_replay(&raw)),
            generation,
            offset,
            alive: true,
        })
    }

    /// The view went away (unmount). Output keeps going to history only.
    pub fn detach(&self, id: &str) -> Result<(), String> {
        let s = self.get(id)?;
        let mut o = s.out();
        o.viewers = o.viewers.saturating_sub(1);
        if o.viewers == 0 {
            o.reset_flow();
        }
        s.central_viewers.store(o.viewers, Ordering::Relaxed);
        drop(o);
        s.cv.notify_all();
        Ok(())
    }

    /// Acknowledges every chunk up to `seq` (the view wrote it).
    pub fn ack(&self, id: &str, seq: u64) -> Result<(), String> {
        let s = self.get(id)?;
        let mut o = s.out();
        while let Some(&(q, len)) = o.inflight.front() {
            if q > seq {
                break;
            }
            o.inflight.pop_front();
            o.inflight_bytes = o.inflight_bytes.saturating_sub(len);
        }
        o.last_ack = Instant::now();
        drop(o);
        s.cv.notify_all();
        Ok(())
    }

    /// Clears the history and starts a new generation (views re-attach).
    pub fn clear(&self, id: &str) -> Result<u64, String> {
        let s = self.get(id)?;
        let mut o = s.out();
        o.history.clear();
        o.generation += 1;
        o.offset = 0;
        o.reset_flow();
        o.disk_due.get_or_insert_with(Instant::now);
        let g = o.generation;
        drop(o);
        s.cv.notify_all();
        {
            let mut t = s.term();
            t.generation = next_screen_gen();
            t.touch();
        }
        self.shared.damage();
        Ok(g)
    }

    /// ⌘K: wipes the screen and the scrollback of the view (and the saved
    /// transcript). The program is not told; its next output starts at the
    /// top.
    pub fn clear_screen(&self, id: &str) -> Result<(), String> {
        self.clear(id)?;
        let s = self.get(id)?;
        {
            let mut t = s.term();
            t.engine.search_clear();
            t.engine.selection_clear();
            t.engine.scroll_display(ScrollBy::Bottom);
            // 2J first: alacritty moves the cleared screen into the
            // scrollback, which 3J then drops.
            t.engine.advance(b"\x1b[H\x1b[2J\x1b[3J");
            let _ = t.engine.take_events();
            t.touch();
        }
        self.shared.damage();
        Ok(())
    }

    /// ⌘⇧B: turns the scroll lock on or off; returns the new state.
    pub fn set_scroll_lock(&self, id: &str, on: Option<bool>) -> Result<bool, String> {
        let s = self.get(id)?;
        let mut t = s.term();
        t.scroll_lock = on.unwrap_or(!t.scroll_lock);
        let now = t.scroll_lock;
        if !now {
            t.engine.scroll_display(ScrollBy::Bottom);
            t.touch();
        }
        drop(t);
        self.shared.damage();
        Ok(now)
    }

    /// How many times `query` (plain text; case-insensitive unless it has
    /// capitals) occurs in the scrollback and the screen.
    pub fn search_count(&self, id: &str, query: &str) -> Result<usize, String> {
        if query.is_empty() {
            return Ok(0);
        }
        let s = self.get(id)?;
        let t = s.term();
        let (_, rows) = t.engine.size();
        let lines = t
            .engine
            .lines_text(-(t.engine.history_len() as i32), rows as i32);
        drop(t);
        Ok(count_matches(&lines, query))
    }

    /// Ends the find: highlight gone, view back at the bottom.
    pub fn search_clear(&self, id: &str) -> Result<(), String> {
        let s = self.get(id)?;
        let mut t = s.term();
        t.engine.search_clear();
        t.engine.scroll_display(ScrollBy::Bottom);
        t.touch();
        drop(t);
        self.shared.damage();
        Ok(())
    }

    /// Kills every session (app exit).
    pub fn shutdown_all(&self) {
        let ids: Vec<String> = lock(&self.sessions).keys().cloned().collect();
        for id in ids {
            let _ = self.close(&id, false);
        }
        let keys: Vec<String> = lock(&self.subs).keys().cloned().collect();
        for k in keys {
            self.unsubscribe_frames(&k);
        }
    }
}

/// A regex that matches `text` literally.
fn literal_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for c in text.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Occurrences of `query` in `lines` (smart case, like the find itself).
fn count_matches(lines: &[String], query: &str) -> usize {
    let smart = !query.chars().any(char::is_uppercase);
    let q = if smart {
        query.to_lowercase()
    } else {
        query.to_string()
    };
    lines
        .iter()
        .map(|l| {
            let l = if smart { l.to_lowercase() } else { l.clone() };
            l.match_indices(q.as_str()).count()
        })
        .sum()
}

fn b64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

// ---------------------------------------------------------------- threads

/// Handles what the engine asked for after an advance or a sync flush.
/// Returns the query answers to write (outside the term lock).
fn drain_engine(s: &Session, t: &mut TermState, sh: &Shared) -> Vec<Vec<u8>> {
    let mut answers = Vec::new();
    for ev in t.engine.take_events() {
        match ev {
            EngineEvent::PtyWrite(b) => answers.push(b),
            EngineEvent::ClipboardStore(text) => {
                if let Some(e) = sh.events() {
                    e.clipboard(&s.id, &text);
                }
            }
            // Titles and bells come from the sniffer (one source of truth).
            EngineEvent::Title(_) | EngineEvent::Bell => {}
        }
    }
    answers
}

fn read_loop(
    s: Arc<Session>,
    sh: Arc<Shared>,
    mut reader: Box<dyn Read + Send>,
    waiter: Option<Box<dyn PtyWaiter>>,
) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let data = &buf[..n];

        // Emulator, sniffer, agent state.
        let (answers, changed, sync_due) = {
            let mut t = s.term();
            let now = Instant::now();
            let events = t.sniffer.feed(data);
            let pinned =
                (t.scroll_lock && t.engine.display_offset() == 0).then(|| t.engine.history_len());
            t.engine.advance(data);
            if let Some(h0) = pinned {
                // Scroll lock at the bottom: keep the lines the user was
                // reading on screen (alacritty already does this when the
                // view is scrolled up).
                let grew = t.engine.history_len().saturating_sub(h0);
                if grew > 0 {
                    t.engine.scroll_display(ScrollBy::Lines(grew as i32));
                }
            }
            let answers = drain_engine(&s, &mut t, &sh);
            t.agent.on_output(now);
            let mut changed = false;
            for ev in &events {
                match ev {
                    OscEvent::Title(title) => t.osc_title = Some(title.clone()),
                    OscEvent::Cwd(c) => t.osc_cwd = Some(c.clone()),
                    _ => {}
                }
                changed |= t.agent.on_osc(ev, now);
            }
            t.touch();
            let sync_due = t.engine.sync_deadline();
            (answers, changed.then(|| t.agent.signal().clone()), sync_due)
        };
        // A Central view (xterm.js) answers queries itself while attached.
        if s.central_viewers.load(Ordering::Relaxed) == 0 {
            for a in &answers {
                let _ = s.write_raw(a);
            }
        }
        if let Some(sig) = changed {
            sh.state(&s.id, &sig);
        } else {
            sh.damage();
        }
        {
            let obs = lock(&s.observers);
            for (_, cb) in obs.iter() {
                cb(data);
            }
        }

        let mut o = s.out();
        // Backpressure: a Central view is attached but behind; hold the program.
        while o.viewers > 0 && o.pending.len() >= PENDING_MAX && !o.closed {
            o =
                s.cv.wait_timeout(o, Duration::from_millis(250))
                    .map(|r| r.0)
                    .unwrap_or_else(|e| e.into_inner().0);
        }
        o.history.push(data);
        if o.viewers > 0 {
            if o.pending.is_empty() {
                o.pending_start = o.offset;
                o.pending_since = Some(Instant::now());
            }
            o.pending.extend_from_slice(data);
        }
        o.offset += n as u64;
        let now = Instant::now();
        o.disk_due.get_or_insert(now + DISK_EVERY);
        if o.fg_due.is_none() {
            let earliest = o.fg_last_run.map(|t| t + FG_EVERY).unwrap_or(now);
            o.fg_due = Some(earliest.max(now + Duration::from_millis(50)));
        }
        o.agent_due.get_or_insert(now + AGENT_EVERY);
        o.sync_due = sync_due;
        drop(o);
        s.cv.notify_all();
    }

    let exit = waiter.map(|mut w| w.wait()).unwrap_or_default();
    {
        let mut o = s.out();
        o.eof = true;
        o.exit_code = exit.code;
        o.exit_signal = exit.signal.clone();
        o.foreground = Foreground::default();
        o.fg_due = None;
        o.agent_due = None;
        o.sync_due = None;
        o.disk_due.get_or_insert_with(Instant::now);
    }
    lock(&s.writer).take();
    s.cv.notify_all();
    let sig = {
        let mut t = s.term();
        if t.engine.sync_deadline().is_some() {
            t.engine.flush_sync();
            let _ = t.engine.take_events();
        }
        t.alive = false;
        t.touch();
        t.agent
            .on_exit(exit.code, exit.signal.as_deref(), Instant::now());
        t.agent.signal().clone()
    };
    if let Some(ev) = sh.events() {
        ev.exit(&s.id, exit.code, exit.signal.as_deref());
    }
    sh.state(&s.id, &sig);
}

fn pump_loop(s: Arc<Session>, sh: Arc<Shared>) {
    let mut o = s.out();
    loop {
        let now = Instant::now();

        // A Central view that stopped acknowledging is dropped; it re-attaches.
        if o.viewers > 0 && o.window_full() && now.duration_since(o.last_ack) >= STALL {
            o.viewers = 0;
            s.central_viewers.store(0, Ordering::Relaxed);
            o.reset_flow();
            drop(o);
            s.cv.notify_all();
            if let Some(ev) = sh.events() {
                ev.stall(&s.id);
            }
            o = s.out();
            continue;
        }

        // Central output.
        if o.viewers > 0 && !o.pending.is_empty() && !o.window_full() {
            let ripe = o
                .pending_since
                .map(|t| now.duration_since(t) >= COALESCE)
                .unwrap_or(true)
                || o.pending.len() >= WINDOW_BYTES / 2
                || o.eof;
            if ripe {
                let room = WINDOW_BYTES.saturating_sub(o.inflight_bytes).max(4096);
                let take = o.pending.len().min(room);
                let chunk: Vec<u8> = o.pending.drain(..take).collect();
                let start = o.pending_start;
                o.pending_start += take as u64;
                o.pending_since = if o.pending.is_empty() {
                    None
                } else {
                    Some(now)
                };
                let seq = o.next_seq;
                o.next_seq += 1;
                if o.inflight.is_empty() {
                    o.last_ack = now;
                }
                o.inflight.push_back((seq, take));
                o.inflight_bytes += take;
                let generation = o.generation;
                drop(o);
                s.cv.notify_all();
                if let Some(ev) = sh.events() {
                    ev.data(&s.id, generation, seq, start, &chunk);
                }
                o = s.out();
                continue;
            }
        }

        // Synchronized update (DEC 2026) that never ended: flush it.
        if o.sync_due.is_some_and(|t| now >= t) {
            o.sync_due = None;
            drop(o);
            let answers = {
                let mut t = s.term();
                let expired = t
                    .engine
                    .sync_deadline()
                    .is_some_and(|d| Instant::now() >= d);
                if expired {
                    t.engine.flush_sync();
                    t.touch();
                    drain_engine(&s, &mut t, &sh)
                } else {
                    Vec::new()
                }
            };
            if s.central_viewers.load(Ordering::Relaxed) == 0 {
                for a in &answers {
                    let _ = s.write_raw(a);
                }
            }
            sh.damage();
            o = s.out();
            continue;
        }

        // History to disk.
        if o.disk_due.is_some_and(|t| now >= t || o.closed) {
            o.disk_due = None;
            let job = o.history.take_job();
            drop(o);
            if let Some(job) = job {
                job.run();
            }
            o = s.out();
            continue;
        }

        // Foreground label.
        if o.fg_due.is_some_and(|t| now >= t) && !o.eof && !o.closed {
            o.fg_due = None;
            o.fg_last_run = Some(now);
            drop(o);
            let fg = s.compute_foreground();
            o = s.out();
            if fg != o.foreground {
                o.foreground = fg.clone();
                o.agent_due.get_or_insert(now);
                drop(o);
                if let Some(ev) = sh.events() {
                    ev.activity(&s.id, &fg);
                }
                o = s.out();
            }
            continue;
        }

        // Agent heuristics.
        if o.agent_due.is_some_and(|t| now >= t) && !o.eof && !o.closed {
            o.agent_due = None;
            let fg = o.foreground.clone();
            drop(o);
            let (changed, again) = {
                let mut t = s.term();
                let now = Instant::now();
                let tail = t
                    .agent
                    .would_accept(SignalSource::Heuristic, now)
                    .then(|| screen::tail(&*t.engine, 8));
                let changed = t.agent.tick(now, &fg, tail.as_deref());
                (
                    changed.then(|| t.agent.signal().clone()),
                    t.agent.wants_tick(now),
                )
            };
            if let Some(sig) = changed {
                sh.state(&s.id, &sig);
            }
            o = s.out();
            if again && !o.eof {
                o.agent_due.get_or_insert(Instant::now() + AGENT_EVERY);
            }
            continue;
        }

        // Done: closed and flushed, or the program ended and nobody is reading.
        let drained = o.closed || o.pending.is_empty() || o.viewers == 0;
        if (o.closed || o.eof) && drained && o.disk_due.is_none() && !o.history.dirty() {
            if o.closed {
                break;
            }
            // Program ended but the session is kept for its history: sleep
            // until something (attach, clear, close) wakes us.
            o = s.cv.wait(o).unwrap_or_else(|e| e.into_inner());
            continue;
        }
        if o.history.dirty() && o.disk_due.is_none() {
            o.disk_due = Some(now + DISK_EVERY);
        }

        // Sleep until the next deadline, or until woken.
        let mut next: Option<Instant> = None;
        let mut consider = |t: Option<Instant>| {
            if let Some(t) = t {
                next = Some(next.map_or(t, |n| n.min(t)));
            }
        };
        if o.viewers > 0 && !o.pending.is_empty() {
            if o.window_full() {
                consider(Some(o.last_ack + STALL));
            } else {
                consider(o.pending_since.map(|t| t + COALESCE));
            }
        }
        consider(o.disk_due);
        consider(o.sync_due);
        if !o.eof {
            consider(o.fg_due);
            consider(o.agent_due);
        }
        o = match next {
            Some(t) => {
                let wait = t
                    .saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(1));
                s.cv.wait_timeout(o, wait)
                    .map(|r| r.0)
                    .unwrap_or_else(|e| e.into_inner().0)
            }
            None => s.cv.wait(o).unwrap_or_else(|e| e.into_inner()),
        };
    }
}

/// One per frame subscriber: waits for damage (or credit), builds a frame
/// with only what changed in its visible terminals, and sends it.
fn ticker(me: Weak<TermManager>, sh: Arc<Shared>, sub: Arc<FrameSub>, key: String) {
    let mut seen = 0u64;
    let mut last_sent: Option<Instant> = None;
    loop {
        if sub.stop.load(Ordering::Relaxed) {
            break;
        }
        let Some(mgr) = me.upgrade() else { break };
        // Pace: at most one frame per FRAME_MIN.
        if let Some(t) = last_sent {
            let since = t.elapsed();
            if since < FRAME_MIN {
                std::thread::sleep(FRAME_MIN - since);
            }
        }
        let now = Instant::now();
        let (frame, retry, credit) = {
            let mut w = lock(&sub.wire);
            if w.can_send(now) {
                let (f, r) = w.build(&*mgr, now);
                (f, r, true)
            } else {
                (None, false, false)
            }
        };
        drop(mgr);
        if let Some(f) = frame {
            if !sub.sink.send(f) {
                sub.stop.store(true, Ordering::Relaxed);
                if let Some(m) = me.upgrade() {
                    let mut subs = lock(&m.subs);
                    if subs.get(&key).is_some_and(|s| Arc::ptr_eq(s, &sub)) {
                        subs.remove(&key);
                    }
                }
                break;
            }
            last_sent = Some(Instant::now());
            // More may be pending right away (damage during the build).
            continue;
        }
        let timeout = if retry {
            wire::MINI_EVERY / 2
        } else if !credit {
            // Waiting for credit: frame_done wakes us; the stall check needs a tick.
            Duration::from_millis(500)
        } else {
            TICK_IDLE
        };
        seen = sh.wait(seen, timeout);
    }
}
