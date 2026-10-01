//! Holding the performance profile (or keeping the machine awake) for a
//! while, and letting go on its own.
//!
//! Two kinds of holders share one system hold:
//!
//! - the manual hold of `wmb_power_hold` (screen or agent), with a deadline
//!   of at most four hours;
//! - automatic holds while OmniGet downloads or runs an agent job
//!   ([`auto_hold_begin`] returns an RAII [`HoldToken`]), counted by
//!   reference and only when the setting is on.
//!
//! The system hold dies with OmniGet on every OS: the D-Bus daemon drops
//! `HoldProfile` when our connection closes, Windows clears a power request
//! when its handle closes with the process, and `caffeinate -w <our pid>`
//! exits when we do. So a crash never leaves the machine held.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// Longest manual hold, in minutes.
pub const MAX_MINUTES: u32 = 240;

/// Seconds since the epoch; tests swap it for a fake clock.
pub trait Clock: Send + Sync {
    fn now(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        super::super::store::now()
    }
}

/// The OS side: hold on `acquire`, let go when the guard drops.
pub trait Backend: Send + Sync {
    fn name(&self) -> &'static str;
    fn acquire(&self, reason: &str, max_secs: Option<u64>) -> Result<Box<dyn Send>, String>;
}

/// What the screen and the agent see.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HoldState {
    pub active: bool,
    /// The manual hold, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Downloads and agent jobs holding right now.
    pub auto_holders: Vec<String>,
    pub auto_enabled: bool,
    pub backend: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

struct Manual {
    started: i64,
    until: i64,
    reason: String,
}

#[derive(Default)]
struct Inner {
    manual: Option<Manual>,
    auto: BTreeMap<u64, String>,
    /// Holders that do not depend on the setting (game mode for a PID).
    tasks: BTreeMap<u64, String>,
    auto_enabled: bool,
    next: u64,
    guard: Option<Box<dyn Send>>,
    error: Option<String>,
}

pub struct Holds {
    backend: Box<dyn Backend>,
    clock: Box<dyn Clock>,
    inner: Mutex<Inner>,
}

impl Holds {
    pub fn new(backend: Box<dyn Backend>, clock: Box<dyn Clock>) -> Holds {
        Holds {
            backend,
            clock,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Start (or extend) the manual hold. Minutes are clamped to 1..=240.
    pub fn hold(&self, minutes: u32, reason: &str) -> Result<HoldState, String> {
        let minutes = minutes.clamp(1, MAX_MINUTES);
        let now = self.clock.now();
        let mut g = self.lock();
        let started = g.manual.as_ref().map(|m| m.started).unwrap_or(now);
        g.manual = Some(Manual {
            started,
            until: now + i64::from(minutes) * 60,
            reason: reason.to_string(),
        });
        // A new reason or deadline: take the system hold again so the
        // backend's own safety timer matches the new deadline.
        g.guard = None;
        if let Err(e) = self.sync(&mut g) {
            g.manual = None;
            return Err(e);
        }
        Ok(self.state_of(&g))
    }

    pub fn release(&self) -> HoldState {
        let mut g = self.lock();
        g.manual = None;
        let _ = self.sync(&mut g);
        self.state_of(&g)
    }

    /// Let go of a manual hold whose time is up. Production calls it from a
    /// timer thread; tests call it after moving the fake clock.
    pub fn tick(&self) -> HoldState {
        let now = self.clock.now();
        let mut g = self.lock();
        if g.manual.as_ref().is_some_and(|m| now >= m.until) {
            g.manual = None;
            let _ = self.sync(&mut g);
        }
        self.state_of(&g)
    }

    pub fn set_auto(&self, on: bool) {
        let mut g = self.lock();
        g.auto_enabled = on;
        if !on {
            g.auto.clear();
        }
        let _ = self.sync(&mut g);
    }

    /// `None` when automatic holds are off.
    pub fn auto_begin(&self, reason: &str) -> Option<u64> {
        let mut g = self.lock();
        if !g.auto_enabled {
            return None;
        }
        g.next += 1;
        let id = g.next;
        g.auto.insert(id, reason.to_string());
        let _ = self.sync(&mut g);
        Some(id)
    }

    /// A holder that ignores the automatic setting (the person asked for
    /// it, e.g. game mode on a PID).
    pub fn task_begin(&self, reason: &str) -> Result<u64, String> {
        let mut g = self.lock();
        g.next += 1;
        let id = g.next;
        g.tasks.insert(id, reason.to_string());
        if let Err(e) = self.sync(&mut g) {
            g.tasks.remove(&id);
            return Err(e);
        }
        Ok(id)
    }

    pub fn auto_end(&self, id: u64) {
        let mut g = self.lock();
        if g.auto.remove(&id).is_some() || g.tasks.remove(&id).is_some() {
            let _ = self.sync(&mut g);
        }
    }

    pub fn state(&self) -> HoldState {
        let g = self.lock();
        self.state_of(&g)
    }

    fn wanted(g: &Inner) -> bool {
        g.manual.is_some() || !g.auto.is_empty() || !g.tasks.is_empty()
    }

    fn sync(&self, g: &mut Inner) -> Result<(), String> {
        if !Self::wanted(g) {
            g.guard = None;
            g.error = None;
            return Ok(());
        }
        if g.guard.is_some() {
            return Ok(());
        }
        let (reason, max) = match &g.manual {
            Some(m) => (
                m.reason.clone(),
                Some((m.until - self.clock.now()).max(1) as u64),
            ),
            None => (
                g.auto
                    .values()
                    .chain(g.tasks.values())
                    .next()
                    .cloned()
                    .unwrap_or_default(),
                None,
            ),
        };
        match self.backend.acquire(&reason, max) {
            Ok(guard) => {
                g.guard = Some(guard);
                g.error = None;
                Ok(())
            }
            Err(e) => {
                g.error = Some(e.clone());
                Err(e)
            }
        }
    }

    fn state_of(&self, g: &Inner) -> HoldState {
        HoldState {
            active: g.guard.is_some(),
            until: g.manual.as_ref().map(|m| m.until),
            started: g.manual.as_ref().map(|m| m.started),
            reason: g.manual.as_ref().map(|m| m.reason.clone()),
            auto_holders: g.auto.values().chain(g.tasks.values()).cloned().collect(),
            auto_enabled: g.auto_enabled,
            backend: self.backend.name().to_string(),
            error: g.error.clone(),
        }
    }
}

// ── The process-wide instance ────────────────────────────────────────

static HOLDS: once_cell::sync::Lazy<Arc<Holds>> =
    once_cell::sync::Lazy::new(|| Arc::new(Holds::new(super::backend(), Box::new(SystemClock))));

pub fn global() -> Arc<Holds> {
    HOLDS.clone()
}

/// Wake up at the deadline and let go; a later hold starts its own timer
/// and the earlier one finds nothing to release (`tick` checks the clock).
pub fn arm_timer(until: i64) {
    let h = global();
    std::thread::Builder::new()
        .name("wmb-power-hold".into())
        .spawn(move || {
            let wait = (until - super::super::store::now()).max(0) as u64;
            std::thread::sleep(std::time::Duration::from_secs(wait + 1));
            let st = h.tick();
            persist(&st);
        })
        .ok();
}

/// Best-effort copy in `kv` for the screen and `wmb_power_status` of other
/// processes (the CLI). Memory is the truth inside the app.
pub fn persist(st: &HoldState) {
    if cfg!(test) {
        return;
    }
    if let Ok(ctx) = super::super::store::Ctx::app() {
        let _ = ctx.kv_set("power_hold", st);
    }
}

/// RAII token of an automatic hold: dropping it lets go.
#[must_use = "the hold ends when the token is dropped"]
pub struct HoldToken {
    id: Option<u64>,
}

impl HoldToken {
    pub fn is_holding(&self) -> bool {
        self.id.is_some()
    }
}

impl Drop for HoldToken {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let h = global();
            h.auto_end(id);
            persist(&h.state());
        }
    }
}

/// Hold while a big download or an agent job runs. Inert when the
/// "hold power during downloads and agents" setting is off.
pub fn auto_hold_begin(reason: &str) -> HoldToken {
    let h = global();
    let id = h.auto_begin(reason);
    if id.is_some() {
        persist(&h.state());
    }
    HoldToken { id }
}

/// Hold for as long as the caller keeps the token, whatever the automatic
/// setting says (the person asked for it). Errors when the system refuses.
pub fn hold_for(reason: &str) -> Result<HoldToken, String> {
    let h = global();
    let id = h.task_begin(reason)?;
    persist(&h.state());
    Ok(HoldToken { id: Some(id) })
}

/// Longest a PID hold may last, in case a PID is reused.
pub const PID_HOLD_MAX_SECS: u64 = 12 * 3600;

/// Hold while process `pid` runs (game mode): a watcher thread checks it
/// every two seconds and lets go when it exits, after
/// [`PID_HOLD_MAX_SECS`], or when `stop` is dropped/sent.
pub fn hold_while_pid(pid: u32, reason: &str) -> Result<std::sync::mpsc::Sender<()>, String> {
    if !pid_alive(pid) {
        return Err(format!("process {pid} is not running"));
    }
    let token = hold_for(reason)?;
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::Builder::new()
        .name("wmb-power-pid-hold".into())
        .spawn(move || {
            let start = std::time::Instant::now();
            while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                rx.recv_timeout(std::time::Duration::from_secs(2))
            {
                if !pid_alive(pid) || start.elapsed().as_secs() >= PID_HOLD_MAX_SECS {
                    break;
                }
            }
            drop(token);
        })
        .map_err(|e| e.to_string())?;
    Ok(tx)
}

/// Is `pid` still running? (Never signals it: `kill(pid, 0)` only checks.)
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if h.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = unsafe { GetExitCodeProcess(h, &mut code) };
        unsafe { CloseHandle(h) };
        ok != 0 && code == 259 // STILL_ACTIVE
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// Mirror of the app setting `washmybelly.auto_hold_power`.
pub fn set_auto_hold(on: bool) {
    global().set_auto(on);
}
