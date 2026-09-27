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
        Holds { backend, clock, inner: Mutex::new(Inner::default()) }
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
        g.manual = Some(Manual { started, until: now + i64::from(minutes) * 60, reason: reason.to_string() });
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

    pub fn auto_end(&self, id: u64) {
        let mut g = self.lock();
        if g.auto.remove(&id).is_some() {
            let _ = self.sync(&mut g);
        }
    }

    pub fn state(&self) -> HoldState {
        let g = self.lock();
        self.state_of(&g)
    }

    fn wanted(g: &Inner) -> bool {
        g.manual.is_some() || !g.auto.is_empty()
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
            Some(m) => (m.reason.clone(), Some((m.until - self.clock.now()).max(1) as u64)),
            None => (g.auto.values().next().cloned().unwrap_or_default(), None),
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
            auto_holders: g.auto.values().cloned().collect(),
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

/// Mirror of the app setting `washmybelly.auto_hold_power`.
pub fn set_auto_hold(on: bool) {
    global().set_auto(on);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    struct FakeClock(Arc<AtomicI64>);
    impl Clock for FakeClock {
        fn now(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    /// Counts live guards: 1 while the system is held.
    struct FakeBackend {
        live: Arc<AtomicUsize>,
        acquired: Arc<AtomicUsize>,
        fail: bool,
    }
    struct Guard(Arc<AtomicUsize>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    impl Backend for FakeBackend {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn acquire(&self, _reason: &str, _max: Option<u64>) -> Result<Box<dyn Send>, String> {
            if self.fail {
                return Err("no daemon".into());
            }
            self.acquired.fetch_add(1, Ordering::SeqCst);
            self.live.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Guard(self.live.clone())))
        }
    }

    fn setup(fail: bool) -> (Holds, Arc<AtomicI64>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let t = Arc::new(AtomicI64::new(1_000));
        let live = Arc::new(AtomicUsize::new(0));
        let acq = Arc::new(AtomicUsize::new(0));
        let h = Holds::new(
            Box::new(FakeBackend { live: live.clone(), acquired: acq.clone(), fail }),
            Box::new(FakeClock(t.clone())),
        );
        (h, t, live, acq)
    }

    #[test]
    fn a_manual_hold_ends_at_its_deadline() {
        let (h, t, live, _) = setup(false);
        let st = h.hold(30, "test").unwrap();
        assert!(st.active);
        assert_eq!(st.until, Some(1_000 + 30 * 60));
        assert_eq!(live.load(Ordering::SeqCst), 1);
        t.store(1_000 + 29 * 60, Ordering::SeqCst);
        assert!(h.tick().active, "still inside the window");
        t.store(1_000 + 30 * 60, Ordering::SeqCst);
        let st = h.tick();
        assert!(!st.active);
        assert_eq!(st.until, None);
        assert_eq!(live.load(Ordering::SeqCst), 0, "the system hold is gone");
    }

    #[test]
    fn minutes_are_clamped_and_release_is_early() {
        let (h, _, live, _) = setup(false);
        assert_eq!(h.hold(10_000, "x").unwrap().until, Some(1_000 + 240 * 60));
        assert_eq!(h.hold(0, "x").unwrap().until, Some(1_000 + 60));
        assert_eq!(live.load(Ordering::SeqCst), 1, "extending keeps one hold");
        assert!(!h.release().active);
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn auto_holds_count_references_and_respect_the_setting() {
        let (h, t, live, acq) = setup(false);
        assert_eq!(h.auto_begin("download"), None, "off by default");
        h.set_auto(true);
        let a = h.auto_begin("download").unwrap();
        let b = h.auto_begin("agent").unwrap();
        assert_eq!(live.load(Ordering::SeqCst), 1);
        assert_eq!(acq.load(Ordering::SeqCst), 1, "one system hold for many holders");
        h.auto_end(a);
        assert_eq!(live.load(Ordering::SeqCst), 1);
        // A manual hold that expires does not drop the automatic one.
        h.hold(1, "m").unwrap();
        t.store(1_000 + 120, Ordering::SeqCst);
        assert!(h.tick().active);
        h.auto_end(b);
        assert_eq!(live.load(Ordering::SeqCst), 0);
        // Turning the setting off lets go of every automatic holder.
        let _c = h.auto_begin("x").unwrap();
        h.set_auto(false);
        assert_eq!(live.load(Ordering::SeqCst), 0);
        assert!(h.state().auto_holders.is_empty());
    }

    #[test]
    fn a_backend_error_is_reported_not_hidden() {
        let (h, _, _, _) = setup(true);
        assert_eq!(h.hold(5, "x").unwrap_err(), "no daemon");
        let st = h.state();
        assert!(!st.active);
        assert_eq!(st.error.as_deref(), Some("no daemon"));
        assert!(!h.release().active);
        assert_eq!(h.state().error, None);
    }
}
