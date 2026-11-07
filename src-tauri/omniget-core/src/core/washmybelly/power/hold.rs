use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

pub const MAX_MINUTES: u32 = 240;

pub trait Clock: Send + Sync {
    fn now(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        super::super::store::now()
    }
}

pub trait Backend: Send + Sync {
    fn name(&self) -> &'static str;
    fn acquire(&self, reason: &str, max_secs: Option<u64>) -> Result<Box<dyn Send>, String>;
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HoldState {
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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
        Holds {
            backend,
            clock,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

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
            Some(m) => (
                m.reason.clone(),
                Some((m.until - self.clock.now()).max(1) as u64),
            ),
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

static HOLDS: once_cell::sync::Lazy<Arc<Holds>> =
    once_cell::sync::Lazy::new(|| Arc::new(Holds::new(super::backend(), Box::new(SystemClock))));

pub fn global() -> Arc<Holds> {
    HOLDS.clone()
}

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

pub fn persist(st: &HoldState) {
    if cfg!(test) {
        return;
    }
    if let Ok(ctx) = super::super::store::Ctx::app() {
        let _ = ctx.kv_set("power_hold", st);
    }
}

#[must_use = "the hold ends when the token is dropped"]
pub struct HoldToken {
    id: Option<u64>,
}

impl HoldToken {}

impl Drop for HoldToken {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let h = global();
            h.auto_end(id);
            persist(&h.state());
        }
    }
}

pub fn auto_hold_begin(reason: &str) -> HoldToken {
    let h = global();
    let id = h.auto_begin(reason);
    if id.is_some() {
        persist(&h.state());
    }
    HoldToken { id }
}

pub fn set_auto_hold(on: bool) {
    global().set_auto(on);
}
