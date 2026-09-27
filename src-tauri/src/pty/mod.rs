//! Embedded terminal of the Central (`portable-pty`).
//!
//! Sessions live in the app process, in a global manager, so closing the panel
//! or the window never kills them: a remounted view calls [`PtyManager::attach`]
//! and gets the replay-safe history plus a cursor, then keeps reading live
//! output from the `pty://data` event.
//!
//! Per session there are two threads:
//! - the **reader** blocks on the PTY, appends to the bounded history and, while
//!   a view is attached, to the pending output;
//! - the **pump** coalesces pending output into ~16 ms blocks, emits them under
//!   a flow-control window (8 chunks / 64 KiB unacknowledged), writes the
//!   history to disk in coalesced batches and refreshes the foreground-program
//!   label. With nothing to do it sleeps on a condvar: no polling at rest.
//!
//! Events (all payloads carry `id`):
//! - `pty://data` `{id, generation, seq, start, data}`: `data` is base64 of the
//!   raw bytes, `start` their absolute offset in the generation. A view drops
//!   bytes before its cursor and re-attaches on a gap. Ack with `pty_ack(seq)`.
//! - `pty://exit` `{id, code, signal}`.
//! - `pty://activity` `{id, running, label, pid}` when the foreground changes.
//! - `pty://stall` `{id}`: the view stopped acking and was detached; attach again.

pub mod foreground;
pub mod history;
pub mod sanitize;

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use foreground::{Foreground, NameCache};
use history::History;

/// Coalescing window for output events.
const COALESCE: Duration = Duration::from_millis(16);
/// Flow-control window: unacknowledged chunks and bytes.
const WINDOW_CHUNKS: usize = 8;
const WINDOW_BYTES: usize = 64 * 1024;
/// Pending output past this blocks the reader (backpressure on the program).
const PENDING_MAX: usize = 1024 * 1024;
/// A full window with no ack for this long detaches the view.
const STALL: Duration = Duration::from_secs(5);
/// History reaches the disk at most this often.
const DISK_EVERY: Duration = Duration::from_millis(500);
/// Foreground label refresh after output, at most this often.
#[cfg(unix)]
const FG_EVERY: Duration = Duration::from_millis(400);
#[cfg(windows)]
const FG_EVERY: Duration = Duration::from_secs(3);
/// Upper bound for sessions alive at once.
const MAX_SESSIONS: usize = 64;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenRequest {
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
    /// Typed into the terminal after start, not submitted (the user presses
    /// Enter). For "log in to X" flows.
    pub initial_input: Option<String>,
    /// History cap in bytes (default 2 MiB, minimum 64 KiB).
    pub history_cap: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
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
}

#[derive(Debug, Clone, Serialize)]
pub struct Attached {
    pub info: Option<SessionInfo>,
    /// base64 of the sanitized history.
    pub history: String,
    pub generation: u64,
    /// Absolute offset the history ends at: live bytes before it are duplicates.
    pub offset: u64,
    /// false when the session is gone and only its transcript remains.
    pub alive: bool,
}

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
    signal: Option<String>,
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
    foreground: Foreground,
    /// Reader hit EOF.
    eof: bool,
    /// Session removed: pump flushes and quits.
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

struct Session {
    id: String,
    title: String,
    cwd: Option<String>,
    command: Vec<String>,
    shell_label: String,
    pid: Option<u32>,
    created_at: i64,
    size: Mutex<(u16, u16)>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    out: Mutex<Out>,
    cv: Condvar,
    names: Mutex<NameCache>,
}

impl Session {
    fn lock(&self) -> MutexGuard<'_, Out> {
        self.out.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn info(&self) -> SessionInfo {
        let (cols, rows) = *self.size.lock().unwrap_or_else(|e| e.into_inner());
        let o = self.lock();
        SessionInfo {
            id: self.id.clone(),
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            command: self.command.clone(),
            pid: self.pid,
            cols,
            rows,
            alive: !o.eof,
            exit_code: o.exit_code,
            exit_signal: o.exit_signal.clone(),
            created_at: self.created_at,
            foreground: o.foreground.clone(),
        }
    }

    fn compute_foreground(&self) -> Foreground {
        let master = self.master.lock().unwrap_or_else(|e| e.into_inner());
        let mut names = self.names.lock().unwrap_or_else(|e| e.into_inner());
        foreground::resolve(&**master, self.pid, &self.shell_label, &mut names)
    }
}

#[derive(Default)]
pub struct PtyManager {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    app: OnceLock<AppHandle>,
    default_cap: Mutex<Option<usize>>,
}

static MANAGER: OnceLock<PtyManager> = OnceLock::new();

/// The process-wide manager. Needs no setup: the first command hands it the
/// `AppHandle` it emits with.
pub fn manager() -> &'static PtyManager {
    MANAGER.get_or_init(PtyManager::default)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn err(code: &str, msg: impl std::fmt::Display) -> String {
    format!("{code}: {msg}")
}

impl PtyManager {
    pub fn bind(&self, app: &AppHandle) {
        let _ = self.app.set(app.clone());
    }

    /// Default history cap for sessions opened from now on.
    pub fn set_history_cap(&self, bytes: usize) {
        *self.default_cap.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(bytes.max(history::MIN_CAP));
    }

    fn get(&self, id: &str) -> Result<Arc<Session>, String> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
            .ok_or_else(|| err("PTY_NOT_FOUND", format!("no terminal {id}")))
    }

    pub fn open(&self, req: OpenRequest) -> Result<SessionInfo, String> {
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
            let map = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
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
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| err("PTY_OPEN", e))?;

        let argv: Vec<String> = req
            .command
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        let (mut cmd, shell_label) = build_command(&argv);
        let cwd = req
            .cwd
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .or_else(dirs::home_dir);
        if let Some(dir) = &cwd {
            cmd.cwd(dir);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "OmniGet");
        cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        #[cfg(target_os = "macos")]
        if std::env::var_os("LANG").is_none() {
            cmd.env("LANG", "en_US.UTF-8");
        }
        for (k, v) in req.env.clone().unwrap_or_default() {
            if !k.is_empty() && !k.contains('=') {
                cmd.env(k, v);
            }
        }

        let child = omniget_core::core::process::spawn_retrying_busy(|| {
            pair.slave
                .spawn_command(cmd.clone())
                .map_err(|e| std::io::Error::other(e.to_string()))
        })
        .map_err(|e| err("PTY_SPAWN", e))?;
        // The child holds the slave now; keeping ours would hide EOF.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| err("PTY_OPEN", e))?;
        let writer = pair.master.take_writer().map_err(|e| err("PTY_OPEN", e))?;
        let pid = child.process_id();
        let killer = child.clone_killer();

        let cap = req
            .history_cap
            .or(*self.default_cap.lock().unwrap_or_else(|e| e.into_inner()))
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

        let session = Arc::new(Session {
            id: id.clone(),
            title,
            cwd: cwd.map(|p| p.display().to_string()),
            command: argv,
            shell_label: shell_label.clone(),
            pid,
            created_at: now_ms(),
            size: Mutex::new((cols, rows)),
            master: Mutex::new(pair.master),
            writer: Mutex::new(Some(writer)),
            killer: Mutex::new(killer),
            out: Mutex::new(Out {
                generation: 1,
                offset: 0,
                history: History::new(cap, log_path),
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
        });

        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), session.clone());

        {
            let s = session.clone();
            let app = self.app.get().cloned();
            std::thread::Builder::new()
                .name(format!("pty-read-{id}"))
                .spawn(move || read_loop(s, reader, child, app))
                .map_err(|e| err("PTY_THREAD", e))?;
        }
        {
            let s = session.clone();
            let app = self.app.get().cloned();
            std::thread::Builder::new()
                .name(format!("pty-pump-{id}"))
                .spawn(move || pump_loop(s, app))
                .map_err(|e| err("PTY_THREAD", e))?;
        }

        if let Some(text) = req.initial_input.filter(|t| !t.is_empty()) {
            // Give the shell a moment to print its prompt first.
            let s = session.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                let _ = write_to(&s, text.as_bytes());
            });
        }
        Ok(session.info())
    }

    pub fn write(&self, id: &str, data: &str) -> Result<(), String> {
        let s = self.get(id)?;
        write_to(&s, data.as_bytes())
    }

    /// "The last one wins": every call applies, whoever sent it.
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        if cols == 0 || rows == 0 {
            return Err(err("PTY_BAD_SIZE", "cols and rows must be positive"));
        }
        let s = self.get(id)?;
        let mut size = s.size.lock().unwrap_or_else(|e| e.into_inner());
        if *size == (cols, rows) {
            return Ok(());
        }
        s.master
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| err("PTY_RESIZE", e))?;
        *size = (cols, rows);
        Ok(())
    }

    /// Kills the program and forgets the session. The transcript stays on disk
    /// unless `delete_history`.
    pub fn close(&self, id: &str, delete_history: bool) -> Result<(), String> {
        let removed = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        if let Some(s) = removed {
            let _ = s.killer.lock().unwrap_or_else(|e| e.into_inner()).kill();
            // Dropping the writer sends EOF for programs that ignore the kill.
            s.writer.lock().unwrap_or_else(|e| e.into_inner()).take();
            let mut o = s.lock();
            o.closed = true;
            if delete_history {
                o.history = History::new(history::MIN_CAP, None);
            }
            drop(o);
            s.cv.notify_all();
        }
        if delete_history {
            if let Some(p) = history::path_for(id) {
                // The pump may still flush once; give it the chance, then remove.
                let _ = std::fs::remove_file(&p);
                std::thread::spawn(move || {
                    std::thread::sleep(DISK_EVERY * 2);
                    let _ = std::fs::remove_file(&p);
                });
            }
        }
        Ok(())
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let sessions: Vec<Arc<Session>> = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        let mut out: Vec<SessionInfo> = sessions.iter().map(|s| s.info()).collect();
        out.sort_by_key(|i| i.created_at);
        out
    }

    /// Two-step attach: returns the sanitized history and the cursor the live
    /// stream continues from, and registers a view so `pty://data` flows.
    /// For an id with no live session, returns its transcript from disk.
    pub fn attach(&self, id: &str) -> Result<Attached, String> {
        let s = match self.get(id) {
            Ok(s) => s,
            Err(e) => {
                let cap = self
                    .default_cap
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .unwrap_or(history::DEFAULT_CAP);
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
            let mut o = s.lock();
            if o.viewers == 0 {
                o.reset_flow();
            }
            o.viewers += 1;
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
        let mut o = s.lock();
        o.viewers = o.viewers.saturating_sub(1);
        if o.viewers == 0 {
            o.reset_flow();
        }
        drop(o);
        s.cv.notify_all();
        Ok(())
    }

    /// Acknowledges every chunk up to `seq` (the view wrote it).
    pub fn ack(&self, id: &str, seq: u64) -> Result<(), String> {
        let s = self.get(id)?;
        let mut o = s.lock();
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
        let mut o = s.lock();
        o.history.clear();
        o.generation += 1;
        o.offset = 0;
        o.reset_flow();
        o.disk_due.get_or_insert_with(Instant::now);
        let g = o.generation;
        drop(o);
        s.cv.notify_all();
        Ok(g)
    }

    /// Foreground program, computed now.
    pub fn foreground(&self, id: &str) -> Result<Foreground, String> {
        let s = self.get(id)?;
        if s.lock().eof {
            return Ok(Foreground::default());
        }
        let fg = s.compute_foreground();
        s.lock().foreground = fg.clone();
        Ok(fg)
    }

    /// Kills every session (app exit).
    pub fn shutdown_all(&self) {
        let ids: Vec<String> = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        for id in ids {
            let _ = self.close(&id, false);
        }
    }
}

fn write_to(s: &Session, data: &[u8]) -> Result<(), String> {
    let mut w = s.writer.lock().unwrap_or_else(|e| e.into_inner());
    let Some(w) = w.as_mut() else {
        return Err(err("PTY_CLOSED", "terminal is closed"));
    };
    w.write_all(data)
        .and_then(|_| w.flush())
        .map_err(|e| err("PTY_WRITE", e))
}

/// Builds the command to spawn and the label of the program that "idles" in it.
fn build_command(argv: &[String]) -> (CommandBuilder, String) {
    if argv.is_empty() {
        let cmd = CommandBuilder::new_default_prog();
        let label = default_shell_label();
        return (cmd, label);
    }
    let label = foreground::label_for(&argv[0]);
    #[cfg(unix)]
    {
        // Through the login shell so the user's PATH applies; `exec "$0" "$@"`
        // avoids any quoting of the argv.
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/bin/sh".to_string());
        let mut cmd = CommandBuilder::new(shell);
        cmd.args(["-l", "-c", "exec \"$0\" \"$@\""]);
        cmd.args(argv);
        (cmd, label)
    }
    #[cfg(windows)]
    {
        let mut cmd = CommandBuilder::new(&argv[0]);
        cmd.args(&argv[1..]);
        (cmd, label)
    }
}

fn default_shell_label() -> String {
    #[cfg(unix)]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
        foreground::label_for(&shell)
    }
    #[cfg(windows)]
    {
        let shell = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
        foreground::label_for(&shell)
    }
}

fn read_loop(
    s: Arc<Session>,
    mut reader: Box<dyn Read + Send>,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    app: Option<AppHandle>,
) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let mut o = s.lock();
        // Backpressure: a view is attached but behind; hold the program.
        while o.viewers > 0 && o.pending.len() >= PENDING_MAX && !o.closed {
            o =
                s.cv.wait_timeout(o, Duration::from_millis(250))
                    .map(|r| r.0)
                    .unwrap_or_else(|e| e.into_inner().0);
        }
        let data = &buf[..n];
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
        drop(o);
        s.cv.notify_all();
    }

    let status = child.wait().ok();
    let (code, signal) = match &status {
        Some(st) => {
            let sig = st.signal().map(|s| s.to_string());
            let code = if sig.is_some() {
                None
            } else {
                Some(st.exit_code() as i32)
            };
            (code, sig)
        }
        None => (None, None),
    };
    {
        let mut o = s.lock();
        o.eof = true;
        o.exit_code = code;
        o.exit_signal = signal.clone();
        o.foreground = Foreground::default();
        o.fg_due = None;
        o.disk_due.get_or_insert_with(Instant::now);
    }
    s.writer.lock().unwrap_or_else(|e| e.into_inner()).take();
    s.cv.notify_all();
    if let Some(app) = app {
        let _ = app.emit(
            "pty://exit",
            ExitEvent {
                id: &s.id,
                code,
                signal,
            },
        );
    }
}

fn pump_loop(s: Arc<Session>, app: Option<AppHandle>) {
    let mut o = s.lock();
    loop {
        let now = Instant::now();

        // A view that stopped acknowledging is dropped; it will re-attach.
        if o.viewers > 0 && o.window_full() && now.duration_since(o.last_ack) >= STALL {
            o.viewers = 0;
            o.reset_flow();
            drop(o);
            s.cv.notify_all();
            if let Some(app) = &app {
                let _ = app.emit("pty://stall", IdEvent { id: &s.id });
            }
            o = s.lock();
            continue;
        }

        // Output.
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
                if let Some(app) = &app {
                    let _ = app.emit(
                        "pty://data",
                        DataEvent {
                            id: &s.id,
                            generation,
                            seq,
                            start,
                            data: b64(&chunk),
                        },
                    );
                }
                o = s.lock();
                continue;
            }
        }

        // History to disk.
        if o.disk_due.is_some_and(|t| now >= t || o.closed) {
            o.disk_due = None;
            let job = o.history.take_job();
            drop(o);
            if let Some(job) = job {
                job.run();
            }
            o = s.lock();
            continue;
        }

        // Foreground label.
        if o.fg_due.is_some_and(|t| now >= t) && !o.eof && !o.closed {
            o.fg_due = None;
            o.fg_last_run = Some(now);
            drop(o);
            let fg = s.compute_foreground();
            o = s.lock();
            if fg != o.foreground {
                o.foreground = fg.clone();
                drop(o);
                if let Some(app) = &app {
                    let _ = app.emit("pty://activity", ActivityEvent { id: &s.id, fg: &fg });
                }
                o = s.lock();
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
        if !o.eof {
            consider(o.fg_due);
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
