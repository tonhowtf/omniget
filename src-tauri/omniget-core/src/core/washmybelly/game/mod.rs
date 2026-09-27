//! Game mode (`wmb_game_mode`): turn on what helps a running game and turn
//! it all off by itself when the game exits.
//!
//! - Linux: gamemode over D-Bus (`com.feralinteractive.GameMode`, or the
//!   `org.freedesktop.portal.GameMode` portal inside Flatpak); the daemon
//!   undoes itself when the game exits. Without gamemode: hold the
//!   `performance` power profile, inhibit the screen saver and lower the
//!   priority of OmniGet's own children.
//! - Windows: a performance power plan, `AboveNormal` priority and EcoQoS off
//!   for the game, the display kept awake; a watcher thread waits on the
//!   process handle and undoes everything.
//! - macOS: Game Mode is automatic and cannot be forced. We only move the
//!   game out of the background band if it is there and put OmniGet's own
//!   children in the background band; a `kqueue` watcher undoes it.
//!
//! One watcher thread per game owns the session: it applies, waits for the
//! exit (or `disable`, or OmniGet closing), and undoes in reverse order.
//! The state is also in `kv` (`game_mode_active`) so a crash is cleaned up
//! on the next start ([`recover`]).

pub mod children;
pub mod dbus;
pub mod parse;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, LazyLock, Mutex, Once};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use super::current_os;
use super::store::{now, Ctx};

pub const KV_KEY: &str = "game_mode_active";

/// One thing to put back, in the order it was done (undone in reverse).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum UndoStep {
    /// gamemode (or the portal) registration. The daemon also drops it by
    /// itself when the game exits.
    GamemodeUnregister {
        portal: bool,
        caller: i32,
        game: i32,
    },
    /// power-profiles-daemon hold; also released when OmniGet's bus
    /// connection closes.
    ReleaseProfile { cookie: u32 },
    /// `org.freedesktop.ScreenSaver` inhibit cookie.
    UnInhibit { cookie: u32 },
    /// Nice value of one of OmniGet's own children, before we raised it.
    Nice { pid: u32, started: u64, prev: i32 },
    /// Linux I/O priority of one of OmniGet's own children, before idle.
    IoPrio { pid: u32, started: u64, prev: i32 },
    /// macOS background band (`PRIO_DARWIN_BG`) to put back.
    DarwinBg {
        pid: u32,
        started: u64,
        restore: bool,
    },
    /// Windows power scheme active before, and the one we set.
    PowerScheme { prev: String, set: String },
    /// Windows priority class of the game before.
    PriorityClass { pid: u32, started: u64, prev: u32 },
    /// Windows EcoQoS state of the game before (`ControlMask`, `StateMask`).
    PowerThrottling {
        pid: u32,
        started: u64,
        control: u32,
        state: u32,
    },
    /// Windows `SetThreadExecutionState` held by the watcher thread.
    ExecutionState,
}

impl UndoStep {
    /// The process a step belongs to: nothing to undo once it is gone.
    fn target(&self) -> Option<(u32, u64)> {
        match *self {
            UndoStep::Nice { pid, started, .. }
            | UndoStep::IoPrio { pid, started, .. }
            | UndoStep::DarwinBg { pid, started, .. }
            | UndoStep::PriorityClass { pid, started, .. }
            | UndoStep::PowerThrottling { pid, started, .. } => Some((pid, started)),
            _ => None,
        }
    }

    pub fn child_pid(&self) -> Option<u32> {
        match *self {
            UndoStep::Nice { pid, .. } | UndoStep::IoPrio { pid, .. } => Some(pid),
            UndoStep::DarwinBg {
                pid,
                restore: false,
                ..
            } => Some(pid),
            _ => None,
        }
    }
}

/// A game with game mode on, as the screen and `kv` see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Active {
    pub pid: u32,
    pub name: String,
    /// Process start time, so a reused pid is never mistaken for the game.
    pub started: u64,
    /// `feral_gamemode`, `portal`, `native`, `windows`, `macos`.
    pub method: String,
    pub since: i64,
    /// Codes of what was turned on (`gamemode`, `power_profile`, ...).
    pub applied: Vec<String>,
    pub notes: Vec<String>,
    pub undo: Vec<UndoStep>,
}

/// What an OS module turned on.
#[derive(Debug, Default)]
pub struct Applied {
    pub method: String,
    pub applied: Vec<String>,
    pub notes: Vec<String>,
    pub undo: Vec<UndoStep>,
}

/// Runs the undo steps (and the periodic re-check) of one system. Tests use
/// a fake that only records.
pub trait Exec: Send + Sync + 'static {
    fn undo(&self, step: &UndoStep) -> Result<String, String>;
    /// About every three seconds while the game runs: new steps (a new
    /// yt-dlp started by OmniGet gets lowered too).
    fn tick(&self, _active: &Active) -> Vec<UndoStep> {
        Vec::new()
    }
    /// Whether the process is still the same one.
    fn alive(&self, pid: u32, started: u64) -> bool {
        alive(pid, started)
    }
}

/// Blocks up to `timeout`; `true` once the process has exited.
pub trait ExitWaiter: Send {
    fn wait(&mut self, timeout: Duration) -> bool;
}

/// Portable waiter: checks the pid and its start time.
pub struct PollWaiter {
    pub pid: u32,
    pub started: u64,
}

impl ExitWaiter for PollWaiter {
    fn wait(&mut self, timeout: Duration) -> bool {
        let step = Duration::from_millis(200);
        let mut left = timeout;
        loop {
            if !alive(self.pid, self.started) {
                return true;
            }
            if left.is_zero() {
                return false;
            }
            let d = left.min(step);
            std::thread::sleep(d);
            left -= d;
        }
    }
}

/// The process `pid` exists and started at `started` (0: any start time).
pub fn alive(pid: u32, started: u64) -> bool {
    let mut sys = System::new();
    let p = Pid::from_u32(pid);
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[p]),
        true,
        ProcessRefreshKind::nothing(),
    );
    match sys.process(p) {
        Some(proc_) => {
            // A zombie waiting to be reaped has already exited.
            if matches!(
                proc_.status(),
                sysinfo::ProcessStatus::Zombie | sysinfo::ProcessStatus::Dead
            ) {
                return false;
            }
            started == 0 || proc_.start_time() == started
        }
        None => false,
    }
}

// ── Which pid may be touched ─────────────────────────────────────────────

/// The game as validated: same user, not OmniGet, not the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub pid: u32,
    pub name: String,
    pub started: u64,
}

const SESSION_NAMES: &[&str] = &[
    "launchd",
    "loginwindow",
    "windowserver",
    "kernel_task",
    "systemd",
    "init",
    "kthreadd",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "smss.exe",
    "lsass.exe",
    "services.exe",
    "dwm.exe",
    "explorer.exe",
    "system",
    "registry",
];

/// Why a pid is refused, or `None`. Pure, so the rules are tested alone.
pub fn refuse_reason(pid: u32, own_pid: u32, name: &str, same_user: bool) -> Option<&'static str> {
    if pid <= 1 {
        return Some("REFUSED: pid 0 and 1 belong to the system");
    }
    if pid == own_pid {
        return Some("REFUSED: that is OmniGet itself");
    }
    if SESSION_NAMES.contains(&name.to_ascii_lowercase().as_str()) {
        return Some("REFUSED: that process keeps the session running");
    }
    if !same_user {
        return Some("REFUSED: the process belongs to another user");
    }
    None
}

pub fn check_pid(pid: u32) -> Result<Target, String> {
    if let Some(r) = refuse_reason(pid, std::process::id(), "", true) {
        return Err(r.into());
    }
    let target = Pid::from_u32(pid);
    let own = Pid::from_u32(std::process::id());
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[target, own]),
        true,
        ProcessRefreshKind::nothing().with_user(UpdateKind::Always),
    );
    let p = sys
        .process(target)
        .ok_or("NOT_FOUND: no process with that pid")?;
    let me = sys.process(own).and_then(|p| p.user_id()).cloned();
    let same = matches!((p.user_id(), me), (Some(a), Some(b)) if *a == b);
    let name = p.name().to_string_lossy().to_string();
    if let Some(r) = refuse_reason(pid, std::process::id(), &name, same) {
        return Err(r.into());
    }
    Ok(Target {
        pid,
        name,
        started: p.start_time(),
    })
}

fn pid_arg(a: &Value) -> Result<u32, String> {
    a["pid"]
        .as_u64()
        .filter(|p| *p <= u32::MAX as u64)
        .map(|p| p as u32)
        .ok_or_else(|| "pid (integer) is required".into())
}

// ── Sessions ─────────────────────────────────────────────────────────────

struct Live {
    id: u64,
    stop: Arc<AtomicBool>,
    active: Arc<Mutex<Active>>,
    done: mpsc::Receiver<Vec<Value>>,
    handle: std::thread::JoinHandle<()>,
}

static LIVE: LazyLock<Mutex<HashMap<u32, Live>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static KV_LOCK: Mutex<()> = Mutex::new(());
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn kv_read(ctx: &Ctx) -> Vec<Active> {
    ctx.kv_get::<Vec<Active>>(KV_KEY).unwrap_or_default()
}

fn kv_put(ctx: Option<&Ctx>, a: &Active) {
    let Some(ctx) = ctx else { return };
    let _g = KV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = kv_read(ctx);
    all.retain(|x| !(x.pid == a.pid && x.since == a.since));
    all.push(a.clone());
    let _ = ctx.kv_set(KV_KEY, &all);
}

fn kv_drop(ctx: Option<&Ctx>, pid: u32, since: i64) {
    let Some(ctx) = ctx else { return };
    let _g = KV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = kv_read(ctx);
    all.retain(|x| !(x.pid == pid && x.since == since));
    let _ = ctx.kv_set(KV_KEY, &all);
}

/// Undo in reverse order; steps of a process that is gone are skipped.
pub fn undo_all(exec: &dyn Exec, steps: &[UndoStep]) -> Vec<Value> {
    steps
        .iter()
        .rev()
        .map(|s| {
            let r = match s.target() {
                Some((pid, started)) if !exec.alive(pid, started) => {
                    Ok("process already ended".to_string())
                }
                _ => exec.undo(s),
            };
            match r {
                Ok(m) => json!({ "step": s, "ok": true, "detail": m }),
                Err(e) => json!({ "step": s, "ok": false, "detail": e }),
            }
        })
        .collect()
}

type ApplyFn = Box<dyn FnOnce() -> Result<Applied, String> + Send>;
type WaiterFn = Box<dyn FnOnce() -> Box<dyn ExitWaiter> + Send>;

const TICK: Duration = Duration::from_secs(1);

/// Start a session: a thread applies, watches and undoes. Returns once the
/// apply step has finished.
pub fn start(
    target: &Target,
    ctx: Option<Ctx>,
    exec: Arc<dyn Exec>,
    apply: ApplyFn,
    waiter: WaiterFn,
) -> Result<Active, String> {
    {
        let live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(l) = live.get(&target.pid) {
            let a = l.active.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if a.started == target.started {
                return Ok(a);
            }
        }
    }
    register_exit_cleanup();
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let stop = Arc::new(AtomicBool::new(false));
    let active = Arc::new(Mutex::new(Active {
        pid: target.pid,
        name: target.name.clone(),
        started: target.started,
        method: String::new(),
        since: now(),
        applied: Vec::new(),
        notes: Vec::new(),
        undo: Vec::new(),
    }));
    let (ready_tx, ready_rx) = mpsc::channel::<Result<Active, String>>();
    let (done_tx, done_rx) = mpsc::channel::<Vec<Value>>();
    let (t_stop, t_active, t_pid) = (stop.clone(), active.clone(), target.pid);

    let handle = std::thread::Builder::new()
        .name(format!("wmb-game-{}", target.pid))
        .spawn(move || {
            let applied = match apply() {
                Ok(a) => a,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let snapshot = {
                let mut a = t_active.lock().unwrap_or_else(|e| e.into_inner());
                a.method = applied.method;
                a.applied = applied.applied;
                a.notes = applied.notes;
                a.undo = applied.undo;
                a.clone()
            };
            kv_put(ctx.as_ref(), &snapshot);
            let _ = ready_tx.send(Ok(snapshot));

            let mut w = waiter();
            let mut n: u32 = 0;
            while !t_stop.load(Ordering::SeqCst) {
                if w.wait(TICK) {
                    break;
                }
                n += 1;
                if n.is_multiple_of(3) && !t_stop.load(Ordering::SeqCst) {
                    let cur = t_active.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    let more = exec.tick(&cur);
                    if !more.is_empty() {
                        let snap = {
                            let mut a = t_active.lock().unwrap_or_else(|e| e.into_inner());
                            a.undo.extend(more);
                            a.clone()
                        };
                        kv_put(ctx.as_ref(), &snap);
                    }
                }
            }
            let fin = t_active.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let report = undo_all(exec.as_ref(), &fin.undo);
            kv_drop(ctx.as_ref(), fin.pid, fin.since);
            {
                let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
                if live.get(&t_pid).is_some_and(|l| l.id == id) {
                    live.remove(&t_pid);
                }
            }
            let _ = done_tx.send(report);
        })
        .map_err(|e| format!("could not start the watcher: {e}"))?;

    match ready_rx.recv_timeout(Duration::from_secs(30)) {
        Ok(Ok(a)) => {
            // The watcher may already have seen the exit and cleaned up; in
            // that case do not leave a dead entry behind.
            let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
            if !handle.is_finished() {
                live.insert(
                    target.pid,
                    Live {
                        id,
                        stop,
                        active,
                        done: done_rx,
                        handle,
                    },
                );
            }
            Ok(a)
        }
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(e)
        }
        Err(_) => {
            stop.store(true, Ordering::SeqCst);
            Err("TIMEOUT: turning game mode on took too long".into())
        }
    }
}

/// Stop one session now and wait for its undo report.
pub fn stop(pid: u32) -> Option<Vec<Value>> {
    let l = LIVE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&pid)?;
    l.stop.store(true, Ordering::SeqCst);
    let report = l
        .done
        .recv_timeout(Duration::from_secs(15))
        .unwrap_or_default();
    let _ = l.handle.join();
    Some(report)
}

pub fn active_list() -> Vec<Active> {
    let live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    let mut v: Vec<Active> = live
        .values()
        .map(|l| l.active.lock().unwrap_or_else(|e| e.into_inner()).clone())
        .collect();
    v.sort_by_key(|a| a.since);
    v
}

/// Undo every session now (OmniGet is closing). Bounded: never blocks the
/// exit for more than a few seconds.
pub fn shutdown() {
    let all: Vec<Live> = LIVE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain()
        .map(|(_, l)| l)
        .collect();
    for l in &all {
        l.stop.store(true, Ordering::SeqCst);
    }
    for l in all {
        let _ = l.done.recv_timeout(Duration::from_secs(4));
    }
}

#[cfg(unix)]
extern "C" fn at_exit() {
    shutdown();
}

fn register_exit_cleanup() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        #[cfg(unix)]
        unsafe {
            libc::atexit(at_exit);
        }
    });
}

/// Undo sessions left in `kv` by an OmniGet that did not close cleanly
/// (sessions of this run are skipped).
pub fn recover_with(ctx: &Ctx, exec: &dyn Exec) -> Vec<Value> {
    let live: Vec<(u32, i64)> = active_list().iter().map(|a| (a.pid, a.since)).collect();
    let stale: Vec<Active> = {
        let _g = KV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let all = kv_read(ctx);
        let (keep, stale): (Vec<Active>, Vec<Active>) = all
            .into_iter()
            .partition(|a| live.contains(&(a.pid, a.since)));
        let _ = ctx.kv_set(KV_KEY, &keep);
        stale
    };
    stale
        .iter()
        .map(|a| json!({ "pid": a.pid, "name": a.name, "report": undo_all(exec, &a.undo) }))
        .collect()
}

/// Once per run: clean what a crash left behind. Also for the app to call
/// at startup.
pub fn recover() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if let Ok(ctx) = Ctx::app() {
            let exec = system_exec();
            let _ = recover_with(&ctx, exec.as_ref());
        }
    });
}

// ── The system's executor ────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn system_exec() -> Arc<dyn Exec> {
    Arc::new(linux::Sys)
}
#[cfg(target_os = "macos")]
fn system_exec() -> Arc<dyn Exec> {
    Arc::new(macos::Sys)
}
#[cfg(target_os = "windows")]
fn system_exec() -> Arc<dyn Exec> {
    Arc::new(windows::Sys)
}

fn enable(target: Target) -> Result<Active, String> {
    let ctx = Ctx::app().ok();
    let exec = system_exec();
    #[cfg(target_os = "linux")]
    let (apply, waiter): (ApplyFn, WaiterFn) = linux::plan(&target);
    #[cfg(target_os = "macos")]
    let (apply, waiter): (ApplyFn, WaiterFn) = macos::plan(&target);
    #[cfg(target_os = "windows")]
    let (apply, waiter): (ApplyFn, WaiterFn) = windows::plan(&target);
    start(&target, ctx, exec, apply, waiter)
}

fn status(pid: Option<u32>) -> Value {
    #[cfg(target_os = "linux")]
    let mut v = linux::status(pid);
    #[cfg(target_os = "macos")]
    let mut v = macos::status(pid);
    #[cfg(target_os = "windows")]
    let mut v = windows::status(pid);
    v["os"] = json!(current_os().as_str());
    let active = active_list();
    if let Some(p) = pid {
        v["pid_active_here"] = json!(active.iter().any(|a| a.pid == p));
    }
    v["active"] = json!(active.iter().map(public).collect::<Vec<_>>());
    v
}

/// What the screen and agents see of a session (no internal undo detail).
fn public(a: &Active) -> Value {
    json!({
        "pid": a.pid,
        "name": a.name,
        "method": a.method,
        "since": a.since,
        "applied": a.applied,
        "notes": a.notes,
        "children_lowered": a.undo.iter().filter_map(UndoStep::child_pid).collect::<std::collections::BTreeSet<_>>().len(),
        "auto_revert": "on_process_exit",
    })
}

pub async fn tool_game_mode(a: Value) -> Result<Value, String> {
    let action = a["action"].as_str().unwrap_or("status").to_string();
    super::tools::blocking(move || {
        recover();
        match action.as_str() {
            "status" => Ok(status(a["pid"].as_u64().map(|p| p as u32))),
            "enable" => {
                let pid = pid_arg(&a)?;
                let target = check_pid(pid)?;
                let act = enable(target)?;
                let mut v = public(&act);
                v["ok"] = json!(true);
                Ok(v)
            }
            "disable" => {
                let pid = pid_arg(&a)?;
                match stop(pid) {
                    Some(report) => Ok(json!({ "ok": true, "pid": pid, "undone": report })),
                    None => Err("NOT_ACTIVE: game mode is not on for that process here".into()),
                }
            }
            other => Err(format!("unknown action {other} (status, enable, disable)")),
        }
    })
    .await
}

/// Actions only the screen may run (`washmybelly_ui` with `game.<action>`):
/// never exposed as MCP tools.
pub async fn ui_action(action: &str, _a: Value) -> Result<Value, String> {
    match action {
        // "Turn everything off" from the screen.
        "disable_all" => {
            super::tools::blocking(|| {
                let pids: Vec<u32> = active_list().iter().map(|a| a.pid).collect();
                let undone: Vec<Value> = pids
                    .into_iter()
                    .map(|p| json!({ "pid": p, "undone": stop(p).unwrap_or_default() }))
                    .collect();
                Ok(json!({ "ok": true, "sessions": undone }))
            })
            .await
        }
        _ => Err(format!("game: unknown screen action {action}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command};
    use std::time::Instant;

    /// Records every undo; knows nothing about the system.
    #[derive(Default)]
    struct Fake {
        undone: Mutex<Vec<UndoStep>>,
        ticks: Mutex<u32>,
    }

    impl Exec for Fake {
        fn undo(&self, step: &UndoStep) -> Result<String, String> {
            self.undone.lock().unwrap().push(step.clone());
            Ok("fake".into())
        }
        fn tick(&self, _a: &Active) -> Vec<UndoStep> {
            *self.ticks.lock().unwrap() += 1;
            Vec::new()
        }
        fn alive(&self, _pid: u32, _started: u64) -> bool {
            true
        }
    }

    fn spawn_sleep(secs: u32) -> Child {
        if cfg!(windows) {
            Command::new("cmd")
                .args(["/C", "timeout", "/T", &secs.to_string(), "/NOBREAK"])
                .spawn()
                .unwrap()
        } else {
            Command::new("sleep").arg(secs.to_string()).spawn().unwrap()
        }
    }

    fn target_of(child: &Child) -> Target {
        let pid = child.id();
        let mut sys = System::new();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            true,
            ProcessRefreshKind::nothing(),
        );
        let started = sys
            .process(Pid::from_u32(pid))
            .map(|p| p.start_time())
            .unwrap_or(0);
        Target {
            pid,
            name: "sleep".into(),
            started,
        }
    }

    fn fake_steps(pid: u32) -> Vec<UndoStep> {
        vec![
            UndoStep::PowerScheme {
                prev: "a".into(),
                set: "b".into(),
            },
            UndoStep::PriorityClass {
                pid,
                started: 0,
                prev: 32,
            },
            UndoStep::ExecutionState,
        ]
    }

    fn apply_with(steps: Vec<UndoStep>) -> ApplyFn {
        Box::new(move || {
            Ok(Applied {
                method: "test".into(),
                applied: vec!["x".into()],
                notes: vec![],
                undo: steps,
            })
        })
    }

    #[test]
    fn refuses_pid_0_1_self_session_and_other_users() {
        assert!(refuse_reason(0, 50, "x", true).is_some());
        assert!(refuse_reason(1, 50, "x", true).is_some());
        assert!(refuse_reason(50, 50, "x", true)
            .unwrap()
            .contains("OmniGet"));
        assert!(refuse_reason(60, 50, "WindowServer", true).is_some());
        assert!(refuse_reason(60, 50, "game", false)
            .unwrap()
            .contains("another user"));
        assert_eq!(refuse_reason(60, 50, "game", true), None);
    }

    #[test]
    fn check_pid_refuses_the_real_pid_1_and_omniget() {
        assert!(check_pid(1).is_err());
        assert!(check_pid(std::process::id())
            .unwrap_err()
            .contains("OmniGet"));
        assert!(check_pid(0).is_err());
    }

    #[test]
    fn check_pid_refuses_a_process_of_another_user() {
        let mut sys = System::new();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_user(UpdateKind::Always),
        );
        let me = sys
            .process(Pid::from_u32(std::process::id()))
            .and_then(|p| p.user_id())
            .cloned();
        let other = sys
            .processes()
            .values()
            .find(|p| p.pid().as_u32() > 1 && p.user_id().is_some() && p.user_id().cloned() != me)
            .map(|p| p.pid().as_u32());
        if let Some(pid) = other {
            let e = check_pid(pid).unwrap_err();
            assert!(e.starts_with("REFUSED"), "{e}");
        }
    }

    #[test]
    fn check_pid_accepts_our_own_test_child() {
        let mut c = spawn_sleep(3);
        let t = check_pid(c.id());
        let _ = c.kill();
        let _ = c.wait();
        assert_eq!(t.unwrap().pid, c.id());
    }

    #[test]
    fn undo_runs_in_reverse_and_skips_dead_processes() {
        struct DeadGame(Mutex<Vec<UndoStep>>);
        impl Exec for DeadGame {
            fn undo(&self, s: &UndoStep) -> Result<String, String> {
                self.0.lock().unwrap().push(s.clone());
                Ok(String::new())
            }
            fn alive(&self, _p: u32, _s: u64) -> bool {
                false
            }
        }
        let e = DeadGame(Mutex::new(vec![]));
        let report = undo_all(&e, &fake_steps(77));
        assert_eq!(report.len(), 3);
        // Priority of a gone game is not touched; the plan and the awake
        // state are put back, last done first.
        assert_eq!(
            *e.0.lock().unwrap(),
            vec![
                UndoStep::ExecutionState,
                UndoStep::PowerScheme {
                    prev: "a".into(),
                    set: "b".into()
                }
            ]
        );
        assert_eq!(report[1]["detail"], "process already ended");
    }

    #[test]
    fn watcher_undoes_everything_when_the_game_exits() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        let child = spawn_sleep(1);
        let t = target_of(&child);
        let fake = Arc::new(Fake::default());
        let (pid, started) = (t.pid, t.started);
        let a = start(
            &t,
            Some(ctx.clone()),
            fake.clone(),
            apply_with(fake_steps(t.pid)),
            Box::new(move || Box::new(PollWaiter { pid, started }) as Box<dyn ExitWaiter>),
        )
        .unwrap();
        assert_eq!(a.method, "test");
        assert_eq!(kv_read(&ctx).len(), 1, "state is in kv while the game runs");
        let mut child = child;
        let _ = child.wait(); // reap, so the pid is really gone
        let t0 = Instant::now();
        while fake.undone.lock().unwrap().len() < 3 && t0.elapsed() < Duration::from_secs(8) {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(fake.undone.lock().unwrap().len(), 3);
        assert_eq!(fake.undone.lock().unwrap()[0], UndoStep::ExecutionState);
        std::thread::sleep(Duration::from_millis(200));
        assert!(!active_list().iter().any(|x| x.pid == pid));
        assert!(kv_read(&ctx).is_empty(), "kv is cleared after the exit");
    }

    #[test]
    fn disable_undoes_early_and_the_game_keeps_running() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        let mut child = spawn_sleep(20);
        let t = target_of(&child);
        let fake = Arc::new(Fake::default());
        let (pid, started) = (t.pid, t.started);
        start(
            &t,
            Some(ctx.clone()),
            fake.clone(),
            apply_with(fake_steps(t.pid)),
            Box::new(move || Box::new(PollWaiter { pid, started }) as Box<dyn ExitWaiter>),
        )
        .unwrap();
        // Starting twice returns the same session.
        let again = start(
            &t,
            None,
            fake.clone(),
            apply_with(vec![]),
            Box::new(move || Box::new(PollWaiter { pid, started }) as Box<dyn ExitWaiter>),
        )
        .unwrap();
        assert_eq!(again.undo.len(), 3);
        let report = stop(t.pid).expect("was active");
        assert_eq!(report.len(), 3);
        assert!(alive(t.pid, t.started), "disable never ends the game");
        assert!(kv_read(&ctx).is_empty());
        assert!(stop(t.pid).is_none());
        let _ = child.kill(); // the test's own child
        let _ = child.wait();
    }

    #[test]
    fn a_failed_apply_leaves_nothing() {
        let mut child = spawn_sleep(5);
        let t = target_of(&child);
        let fake = Arc::new(Fake::default());
        let (pid, started) = (t.pid, t.started);
        let r = start(
            &t,
            None,
            fake.clone(),
            Box::new(|| Err("NOTHING_APPLIED".into())),
            Box::new(move || Box::new(PollWaiter { pid, started }) as Box<dyn ExitWaiter>),
        );
        assert!(r.is_err());
        assert!(!active_list().iter().any(|x| x.pid == t.pid));
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn recover_undoes_what_a_crash_left_in_kv() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        let left = Active {
            pid: 999_999,
            name: "old".into(),
            started: 1,
            method: "windows".into(),
            since: 5,
            applied: vec![],
            notes: vec![],
            undo: vec![
                UndoStep::PowerScheme {
                    prev: "p".into(),
                    set: "s".into(),
                },
                UndoStep::Nice {
                    pid: 999_999,
                    started: 1,
                    prev: 0,
                },
            ],
        };
        ctx.kv_set(KV_KEY, &vec![left]).unwrap();
        let fake = Fake::default();
        let r = recover_with(&ctx, &fake);
        assert_eq!(r.len(), 1);
        assert_eq!(fake.undone.lock().unwrap().len(), 2);
        assert!(kv_read(&ctx).is_empty());
    }

    #[test]
    fn undo_steps_round_trip_through_kv_json() {
        let s = vec![
            UndoStep::GamemodeUnregister {
                portal: false,
                caller: 1,
                game: 2,
            },
            UndoStep::DarwinBg {
                pid: 3,
                started: 4,
                restore: true,
            },
            UndoStep::PowerThrottling {
                pid: 3,
                started: 4,
                control: 4,
                state: 4,
            },
        ];
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("\"step\":\"gamemode_unregister\""));
        let back: Vec<UndoStep> = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn child_pids_are_counted_once() {
        let a = Active {
            pid: 1,
            name: String::new(),
            started: 0,
            method: String::new(),
            since: 0,
            applied: vec![],
            notes: vec![],
            undo: vec![
                UndoStep::Nice {
                    pid: 9,
                    started: 0,
                    prev: 0,
                },
                UndoStep::IoPrio {
                    pid: 9,
                    started: 0,
                    prev: 0,
                },
                UndoStep::DarwinBg {
                    pid: 8,
                    started: 0,
                    restore: false,
                },
            ],
        };
        assert_eq!(public(&a)["children_lowered"], 2);
    }

    #[test]
    fn tick_is_called_while_the_game_runs() {
        let mut child = spawn_sleep(20);
        let t = target_of(&child);
        let fake = Arc::new(Fake::default());
        let (pid, started) = (t.pid, t.started);
        start(
            &t,
            None,
            fake.clone(),
            apply_with(vec![]),
            Box::new(move || Box::new(PollWaiter { pid, started }) as Box<dyn ExitWaiter>),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(3600));
        stop(t.pid);
        let _ = child.kill();
        let _ = child.wait();
        assert!(*fake.ticks.lock().unwrap() >= 1);
    }
}
