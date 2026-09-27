//! The pollers, the shared snapshot and the `limits://state` event.
//!
//! One task, alive only while the strip window exists. Every tick it drains
//! the bus into the activity tracker, fills the `omniget` ring from the app's
//! own telemetry, and starts a read for each provider that is switched on and
//! due. The rules that protect the user are pure functions with tests:
//!
//!   - [`due`]: nothing is due while the window is closed, the master switch
//!     is off, or the provider's own switch is off;
//!   - [`next_delay`]: never sooner than [`MIN_INTERVAL`], doubling on every
//!     failure, and never sooner than a provider's `Retry-After`;
//!   - [`may_refresh`]: a manual refresh cannot be used to hammer a provider.

use super::activity::{Activity, Tracker, EXTERNAL_WINDOW_SECS};
use super::alerts::{self, Alert};
use super::prefs::{Edge, Look, StripPrefs};
use super::providers::{self, OMNIGET_ID};
use super::{now_ms, LimitWindow, ReadError, Reading, UsageProvider, EVENT_CHIME, EVENT_STATE};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

/// No provider is asked more often than this, whatever its reader says.
pub const MIN_INTERVAL: Duration = Duration::from_secs(300);
/// Where the doubling stops. A longer `Retry-After` is still honoured.
pub const MAX_BACKOFF: Duration = Duration::from_secs(3600);
/// A manual refresh is ignored when the last read is younger than this.
pub const MANUAL_FLOOR_MS: i64 = 60_000;
const TICK: Duration = Duration::from_secs(5);
/// The session logs are stat'ed every other tick.
const EXTERNAL_EVERY: u64 = 2;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ring {
    pub id: String,
    pub label: String,
    pub local: bool,
    pub beta: bool,
    /// Which login this ring reads (account name and e-mail), when the tool
    /// can hold several.
    pub account: Option<super::AccountTag>,
    /// `pending`, `ok`, `absent`, `needs_auth`, `rate_limited`, `error`.
    pub status: &'static str,
    pub message: Option<String>,
    pub reading: Option<Reading>,
    pub read_at: Option<i64>,
    pub next_at: Option<i64>,
    pub activity: Activity,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Snapshot {
    pub open: bool,
    pub edge: Edge,
    /// Size, labels, pace tick and contrast, from the prefs.
    pub look: Look,
    pub rings: Vec<Ring>,
}

#[derive(Debug, Clone, Serialize)]
struct Chime {
    provider: String,
    alerts: Vec<Alert>,
}

// ---------------------------------------------------------------------------
// The rules
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Slot {
    /// Epoch ms of the next allowed read; 0 = as soon as the strip is open.
    pub next_at: i64,
    pub failures: u32,
    pub read_at: Option<i64>,
    pub in_flight: bool,
    /// Set while a provider asked us to stay away.
    pub rate_limited: bool,
}

/// Wait before the next read and the new failure count.
pub fn next_delay(
    interval: Duration,
    outcome: Result<(), &ReadError>,
    failures: u32,
) -> (Duration, u32) {
    let base = interval.max(MIN_INTERVAL);
    let Err(error) = outcome else {
        return (base, 0);
    };
    let failures = failures.saturating_add(1);
    let doubled = base
        .saturating_mul(2u32.saturating_pow(failures.min(16)))
        .min(MAX_BACKOFF.max(base));
    let asked = match error {
        ReadError::RateLimited(secs) => Duration::from_secs(*secs),
        _ => Duration::ZERO,
    };
    (doubled.max(asked), failures)
}

/// The providers to read now, in the user's order.
pub fn due(
    prefs: &StripPrefs,
    slots: &HashMap<String, Slot>,
    window_open: bool,
    now: i64,
) -> Vec<String> {
    if !window_open || !prefs.enabled {
        return Vec::new();
    }
    prefs
        .providers
        .iter()
        .filter(|p| p.enabled && p.id != OMNIGET_ID)
        .filter(|p| {
            slots
                .get(&p.id)
                .map(|s| !s.in_flight && s.next_at <= now)
                .unwrap_or(true)
        })
        .map(|p| p.id.clone())
        .collect()
}

/// Whether a manual refresh may pull the next read forward.
pub fn may_refresh(slot: &Slot, now: i64) -> bool {
    !slot.in_flight
        && !slot.rate_limited
        && slot
            .read_at
            .map(|t| now - t >= MANUAL_FLOOR_MS)
            .unwrap_or(true)
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct Held {
    status: Option<&'static str>,
    message: Option<String>,
    reading: Option<Reading>,
    /// `(window id, resets_at)` of the pace warnings already raised: one per
    /// window period.
    paced: Vec<(String, i64)>,
}

#[derive(Default)]
struct Inner {
    prefs: StripPrefs,
    /// The readers as of the last look at `accounts.json`.
    readers: Vec<Arc<dyn UsageProvider>>,
    slots: HashMap<String, Slot>,
    held: HashMap<String, Held>,
    tracker: Tracker,
    cancel: Option<CancellationToken>,
    last_emitted: Option<Snapshot>,
    /// The menu bar usage icon (`crate::usage_tray`): while it is on, the task
    /// stays alive without the strip window and also reads the rings it lists.
    tray_on: bool,
    tray_ids: Vec<String>,
    last_tray: Option<Snapshot>,
}

/// Whether a ring is read: the strip has it on, or the usage icon lists it.
fn reads(g: &Inner, id: &str) -> bool {
    g.prefs.is_on(id) || (g.tray_on && id != OMNIGET_ID && g.tray_ids.iter().any(|t| t == id))
}

/// The task has someone to serve: the strip window, or the usage icon.
fn alive(app: &AppHandle) -> bool {
    window_open(app) || inner().tray_on
}

fn inner() -> MutexGuard<'static, Inner> {
    static INNER: OnceLock<Mutex<Inner>> = OnceLock::new();
    INNER
        .get_or_init(|| Mutex::new(Inner::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn window_open(app: &AppHandle) -> bool {
    app.get_webview_window(super::commands::WINDOW_LABEL)
        .is_some()
}

/// Ids and defaults for `StripPrefs::adopt`: only what never leaves this
/// machine starts ticked, and a newly saved account (`claude:<id>`) starts
/// like the ring of its CLI (`claude`) is set now.
pub fn known(readers: &[Arc<dyn UsageProvider>], prefs: &StripPrefs) -> Vec<(String, bool)> {
    let mut out = vec![(OMNIGET_ID.to_string(), true)];
    out.extend(readers.iter().map(|p| {
        let id = p.id();
        let on = match id.split_once(':') {
            Some((base, _)) => prefs.provider(base).map(|b| b.enabled).unwrap_or(false),
            None => p.local(),
        };
        (id.to_string(), on)
    }));
    out
}

fn build_snapshot(inner: &Inner, open: bool, now: i64) -> Snapshot {
    build_snapshot_for(inner, open, now, false)
}

/// `for_tray`: the rings the usage icon lists instead of the strip's.
fn build_snapshot_for(inner: &Inner, open: bool, now: i64, for_tray: bool) -> Snapshot {
    let readers = &inner.readers;
    let rings = inner
        .prefs
        .providers
        .iter()
        .filter(|p| {
            if for_tray {
                inner.tray_on && inner.tray_ids.contains(&p.id)
            } else {
                inner.prefs.is_on(&p.id)
            }
        })
        .filter_map(|p| {
            let reader = readers.iter().find(|r| r.id() == p.id);
            if reader.is_none() && p.id != OMNIGET_ID {
                return None;
            }
            let held = inner.held.get(&p.id).cloned().unwrap_or_default();
            let slot = inner.slots.get(&p.id);
            Some(Ring {
                id: p.id.clone(),
                label: reader.map(|r| r.label()).unwrap_or("OmniGet").to_string(),
                local: reader.map(|r| r.local()).unwrap_or(true),
                beta: reader.map(|r| r.beta()).unwrap_or(false),
                account: reader.and_then(|r| r.account().cloned()),
                status: held.status.unwrap_or("pending"),
                message: held.message,
                reading: held.reading,
                read_at: slot.and_then(|s| s.read_at),
                next_at: slot.map(|s| s.next_at).filter(|t| *t > 0),
                activity: inner.tracker.of(&p.id, now),
            })
        })
        .collect();
    Snapshot {
        open,
        edge: inner.prefs.edge,
        look: inner.prefs.look(),
        rings,
    }
}

pub fn snapshot(app: &AppHandle) -> Snapshot {
    build_snapshot(&inner(), window_open(app), now_ms())
}

fn emit_if_changed(app: &AppHandle) {
    let open = window_open(app);
    let (strip, tray) = {
        let mut g = inner();
        let now = now_ms();
        let snap = build_snapshot(&g, open, now);
        let strip = (g.last_emitted.as_ref() != Some(&snap)).then(|| {
            g.last_emitted = Some(snap.clone());
            snap
        });
        let tray = if g.tray_on {
            let snap = build_snapshot_for(&g, open, now, true);
            (g.last_tray.as_ref() != Some(&snap)).then(|| {
                g.last_tray = Some(snap.clone());
                snap
            })
        } else {
            None
        };
        (strip, tray)
    };
    if let Some(snap) = strip {
        let _ = app.emit(EVENT_STATE, &snap);
    }
    if let Some(snap) = tray {
        crate::usage_tray::on_state(app, &snap);
    }
}

/// The rings the usage icon lists, for its panel and its first paint.
pub fn tray_snapshot(app: &AppHandle) -> Snapshot {
    build_snapshot_for(&inner(), window_open(app), now_ms(), true)
}

/// Turns the usage icon's demand on or off and says which rings it wants.
/// On, the task starts (or keeps running) even with the strip closed; off,
/// the rings only it wanted are forgotten, and the task stops when the strip
/// is closed too.
pub fn set_tray(app: &AppHandle, on: bool, ids: Vec<String>) {
    {
        let mut g = inner();
        g.tray_on = on;
        g.tray_ids = ids;
        g.last_tray = None;
        forget_off(&mut g);
    }
    if on {
        start(app);
        wake();
    } else if !window_open(app) {
        stop();
    }
    emit_if_changed(app);
}

/// The engine's copy of the prefs. Readings of anything that is now off are
/// forgotten on the spot.
pub fn set_prefs(app: &AppHandle, prefs: StripPrefs) {
    {
        let mut g = inner();
        g.prefs = prefs;
        forget_off(&mut g);
    }
    emit_if_changed(app);
}

fn forget_off(g: &mut Inner) {
    let off: Vec<String> = g
        .held
        .keys()
        .chain(g.slots.keys())
        .filter(|id| !reads(g, id))
        .cloned()
        .collect();
    for id in off {
        g.held.remove(&id);
        g.slots.remove(&id);
    }
}

/// Takes a fresh reader list (an account was added or removed in the app)
/// and lists newcomers in the prefs. Returns whether the prefs changed.
fn take_readers(g: &mut Inner, readers: Vec<Arc<dyn UsageProvider>>) -> bool {
    let known = known(&readers, &g.prefs);
    g.readers = readers;
    let changed = g.prefs.adopt(&known);
    if changed {
        forget_off(g);
    }
    changed
}

/// Pulls the next read of one provider (or of all) forward. Returns the ids
/// that will actually be read; the others were read too recently.
pub fn refresh(provider_id: Option<&str>) -> Vec<String> {
    let now = now_ms();
    let mut g = inner();
    let ids: Vec<String> = g
        .prefs
        .providers
        .iter()
        .filter(|p| provider_id.map(|id| id == p.id).unwrap_or(true))
        .filter(|p| reads(&g, &p.id) && p.id != OMNIGET_ID)
        .map(|p| p.id.clone())
        .collect();
    let mut out = Vec::new();
    for id in ids {
        let slot = g.slots.entry(id.clone()).or_default();
        if may_refresh(slot, now) {
            slot.next_at = 0;
            out.push(id);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The OmniGet ring
// ---------------------------------------------------------------------------

/// OmniGet's own agents: one window per saved account, from the telemetry the
/// app already keeps in memory. No file, no network.
pub fn omniget_reading(t: &crate::llm_manager::TelemetrySnapshot) -> Reading {
    let windows: Vec<LimitWindow> = t
        .quotas
        .iter()
        .map(|q| LimitWindow {
            id: q.id.clone(),
            label: q.label.clone(),
            used: Some(q.used.max(0.0)),
            resets_at: q.resets_at_ms.map(|ms| ms as i64),
            group: (q.source != "reported").then(|| "Estimated".to_string()),
            ..Default::default()
        })
        .collect();
    let spend = t
        .cost_by_day
        .last()
        .map(|b| format!("today: ${:.2} in {} calls", b.cost_usd, b.calls));
    Reading {
        note: spend.or_else(|| windows.is_empty().then(|| "no account yet".to_string())),
        windows,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// The task
// ---------------------------------------------------------------------------

fn apply(
    app: &AppHandle,
    reader: &Arc<dyn UsageProvider>,
    outcome: Option<Result<Reading, ReadError>>,
) {
    let id = reader.id();
    let now = now_ms();
    let mut chime = None;
    {
        let mut g = inner();
        if !reads(&g, id) {
            // Switched off while the read was in the air: the answer is dropped.
            g.slots.remove(id);
            g.held.remove(id);
            return;
        }
        let failures = g.slots.get(id).map(|s| s.failures).unwrap_or(0);
        let (delay, failures) = next_delay(
            reader.poll_interval(),
            match &outcome {
                Some(Err(e)) => Err(e),
                _ => Ok(()),
            },
            failures,
        );
        g.slots.insert(
            id.to_string(),
            Slot {
                next_at: now + delay.as_millis() as i64,
                failures,
                read_at: Some(now),
                in_flight: false,
                rate_limited: matches!(outcome, Some(Err(ReadError::RateLimited(_)))),
            },
        );
        let held = g.held.entry(id.to_string()).or_default().clone();
        let next = match outcome {
            None => Held {
                status: Some("absent"),
                ..Default::default()
            },
            Some(Ok(reading)) => {
                let mut raised = alerts::between(
                    held.reading.as_ref(),
                    &reading,
                    &g.prefs.sane_thresholds(),
                    now,
                );
                // Periods that ended are dropped; the new ones warned about
                // join, whether or not the user wants to hear it, so turning
                // the switch on later does not replay an old forecast.
                let mut paced: Vec<(String, i64)> = held
                    .paced
                    .iter()
                    .filter(|(_, r)| *r > now)
                    .cloned()
                    .collect();
                // No forecast on a first reading: opening the strip halfway
                // through a heavy session is not news.
                if held.reading.is_some() {
                    for a in alerts::pace(&reading, &paced, now) {
                        if let alerts::Alert::Pace { window, .. } = &a {
                            if let Some(r) = reading
                                .windows
                                .iter()
                                .find(|w| &w.id == window)
                                .and_then(|w| w.resets_at)
                            {
                                paced.push((window.clone(), r));
                            }
                        }
                        raised.push(a);
                    }
                }
                let found = alerts::wanted(
                    raised,
                    g.prefs.notify_thresholds,
                    g.prefs.notify_reset,
                    g.prefs.notify_pace,
                );
                if !found.is_empty() && !g.prefs.is_muted(id) {
                    chime = Some(found);
                }
                Held {
                    status: Some("ok"),
                    message: None,
                    reading: Some(reading),
                    paced,
                }
            }
            // The last good reading stays on the card next to the error.
            Some(Err(e)) => Held {
                status: Some(e.code()),
                message: Some(e.message()),
                reading: held.reading,
                paced: held.paced,
            },
        };
        g.held.insert(id.to_string(), next);
    }
    if let Some(found) = chime {
        use tauri_plugin_notification::NotificationExt;
        if let Some((title, body)) = alerts::summary(&reader.display_label(), &found) {
            let _ = app.notification().builder().title(title).body(body).show();
        }
        let _ = app.emit(
            EVENT_CHIME,
            Chime {
                provider: id.to_string(),
                alerts: found,
            },
        );
    }
    emit_if_changed(app);
}

fn spawn_read(app: AppHandle, reader: Arc<dyn UsageProvider>) {
    tauri::async_runtime::spawn(async move {
        let outcome = if reader.detect().await {
            Some(reader.read().await)
        } else {
            None
        };
        apply(&app, &reader, outcome);
    });
}

/// Starts the task if it is not running. Called once the window exists.
pub fn start(app: &AppHandle) {
    if inner().cancel.is_some() {
        return;
    }
    // Disk first, lock after: `start` can run on the main thread (the usage
    // icon's menu, app setup), and every other caller of `inner()` would wait
    // behind a file read.
    let readers = providers::cached();
    let prefs = super::prefs::load();
    let cancel = {
        let mut g = inner();
        if g.cancel.is_some() {
            return;
        }
        g.prefs = prefs;
        take_readers(&mut g, readers);
        let token = CancellationToken::new();
        g.cancel = Some(token.clone());
        token
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let llm = app.try_state::<crate::AppState>().map(|s| s.llm.clone());
        let mut bus = llm.as_ref().map(|m| m.bus().subscribe());
        let mut tick: u64 = 0;
        loop {
            if cancel.is_cancelled() || !alive(&app) {
                break;
            }
            if tick > 0 && tick.is_multiple_of(EXTERNAL_EVERY) {
                // Accounts created or removed in the app show up (or leave)
                // here, without a restart.
                if let Ok(fresh) = tokio::task::spawn_blocking(providers::all).await {
                    let (changed, prefs) = {
                        let mut g = inner();
                        let changed = take_readers(&mut g, fresh);
                        (changed, g.prefs.clone())
                    };
                    if changed {
                        let _ = app.emit(super::EVENT_PREFS, &prefs);
                        super::commands::reflow(&app);
                    }
                }
            }
            let now = now_ms();
            let mut events = Vec::new();
            if let Some(rx) = bus.as_mut() {
                use tokio::sync::broadcast::error::TryRecvError;
                loop {
                    match rx.try_recv() {
                        Ok(ev) => events.push(ev),
                        Err(TryRecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            }
            let (watch_clis, omniget_on) = {
                let g = inner();
                (
                    g.prefs.is_on("claude") || g.prefs.is_on("codex"),
                    g.prefs.is_on(OMNIGET_ID),
                )
            };
            let external = if tick % EXTERNAL_EVERY == 0 {
                Some(if watch_clis {
                    tokio::task::spawn_blocking(|| {
                        omniget_core::core::llm::cli_usage::activity::poll(Duration::from_secs(
                            EXTERNAL_WINDOW_SECS,
                        ))
                    })
                    .await
                    .unwrap_or_default()
                } else {
                    Vec::new()
                })
            } else {
                None
            };
            let own = (omniget_on)
                .then(|| llm.as_ref().map(|m| omniget_reading(&m.telemetry())))
                .flatten();
            let to_read = {
                let mut g = inner();
                for ev in &events {
                    g.tracker.note_bus(ev, now);
                }
                if let Some(active) = &external {
                    let shown: Vec<_> = active
                        .iter()
                        .filter(|a| {
                            super::activity::ring_of_cli(a.cli)
                                .map(|ring| g.prefs.is_on(ring))
                                .unwrap_or(false)
                        })
                        .cloned()
                        .collect();
                    g.tracker.note_external(&shown, now);
                }
                match own {
                    Some(reading) => {
                        g.held.insert(
                            OMNIGET_ID.to_string(),
                            Held {
                                status: Some("ok"),
                                reading: Some(reading),
                                ..Default::default()
                            },
                        );
                    }
                    None => {
                        g.held.remove(OMNIGET_ID);
                    }
                }
                let mut ids = due(&g.prefs, &g.slots, window_open(&app), now);
                if g.tray_on {
                    // The usage icon's rings the strip does not read itself.
                    for id in &g.tray_ids {
                        let free = g
                            .slots
                            .get(id)
                            .map(|s| !s.in_flight && s.next_at <= now)
                            .unwrap_or(true);
                        if free && id != OMNIGET_ID && !ids.contains(id) {
                            ids.push(id.clone());
                        }
                    }
                }
                for id in &ids {
                    g.slots.entry(id.clone()).or_default().in_flight = true;
                }
                ids
            };
            let readers = inner().readers.clone();
            for id in to_read {
                match readers.iter().find(|r| r.id() == id) {
                    Some(reader) => spawn_read(app.clone(), reader.clone()),
                    // Listed in the prefs but gone from the accounts.
                    None => {
                        if let Some(slot) = inner().slots.get_mut(&id) {
                            slot.in_flight = false;
                        }
                    }
                }
            }
            emit_if_changed(&app);
            tick += 1;
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = WAKE.notified() => {}
                _ = tokio::time::sleep(TICK) => {}
            }
        }
        // Left because the window went away on its own (not through `stop`,
        // which cancels the token first): forget everything just the same.
        if !cancel.is_cancelled() {
            stop();
        }
    });
}

/// Wakes the task before its next tick: a switch flipped, a refresh asked.
pub fn wake() {
    WAKE.notify_one();
}

static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Stops the task and forgets every reading: a closed strip holds nothing.
pub fn stop() {
    let mut g = inner();
    if g.tray_on {
        // The usage icon still wants its rings: the task keeps running and
        // only what the strip alone read is forgotten.
        g.last_emitted = None;
        forget_off(&mut g);
        return;
    }
    if let Some(token) = g.cancel.take() {
        token.cancel();
    }
    g.held.clear();
    g.slots.clear();
    g.tracker = Tracker::default();
    g.last_emitted = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits_strip::prefs::ProviderPref;

    fn prefs(on: &[&str], off: &[&str]) -> StripPrefs {
        let mut p = StripPrefs {
            enabled: true,
            ..Default::default()
        };
        for (ids, enabled) in [(on, true), (off, false)] {
            for id in ids {
                p.providers.push(ProviderPref {
                    id: (*id).to_string(),
                    enabled,
                    muted: false,
                });
            }
        }
        p
    }

    #[test]
    fn nothing_is_due_while_the_window_is_closed_or_the_master_is_off() {
        let p = prefs(&["claude", "ollama"], &[]);
        assert!(due(&p, &HashMap::new(), false, 0).is_empty());
        let mut off = p.clone();
        off.enabled = false;
        assert!(due(&off, &HashMap::new(), true, 0).is_empty());
        assert_eq!(due(&p, &HashMap::new(), true, 0), ["claude", "ollama"]);
    }

    #[test]
    fn a_provider_that_is_off_is_never_due() {
        let p = prefs(&["ollama"], &["claude", "codex"]);
        assert_eq!(due(&p, &HashMap::new(), true, i64::MAX), ["ollama"]);
        assert!(due(&prefs(&[], &["claude"]), &HashMap::new(), true, i64::MAX).is_empty());
        // The app's own ring has no reader to run.
        assert!(due(&prefs(&[OMNIGET_ID], &[]), &HashMap::new(), true, 0).is_empty());
    }

    #[test]
    fn a_read_in_the_air_or_not_yet_due_is_left_alone() {
        let p = prefs(&["claude", "codex"], &[]);
        let mut slots = HashMap::new();
        slots.insert(
            "claude".to_string(),
            Slot {
                in_flight: true,
                ..Default::default()
            },
        );
        slots.insert(
            "codex".to_string(),
            Slot {
                next_at: 1_000,
                ..Default::default()
            },
        );
        assert!(due(&p, &slots, true, 999).is_empty());
        assert_eq!(due(&p, &slots, true, 1_000), ["codex"]);
    }

    #[test]
    fn no_reader_is_asked_more_often_than_every_five_minutes() {
        assert_eq!(
            next_delay(Duration::from_secs(1), Ok(()), 3),
            (MIN_INTERVAL, 0)
        );
        assert_eq!(
            next_delay(Duration::from_secs(900), Ok(()), 0).0,
            Duration::from_secs(900)
        );
    }

    #[test]
    fn failures_back_off_exponentially_up_to_a_ceiling() {
        let rl = ReadError::RateLimited(60);
        let (d1, f1) = next_delay(MIN_INTERVAL, Err(&rl), 0);
        let (d2, f2) = next_delay(MIN_INTERVAL, Err(&rl), f1);
        let (d3, _) = next_delay(MIN_INTERVAL, Err(&rl), f2);
        assert_eq!(
            (d1, d2, d3),
            (MIN_INTERVAL * 2, MIN_INTERVAL * 4, MIN_INTERVAL * 8)
        );
        assert_eq!(next_delay(MIN_INTERVAL, Err(&rl), 40).0, MAX_BACKOFF);
        // A provider that asks for longer than the ceiling gets it.
        let long = ReadError::RateLimited(7_200);
        assert_eq!(
            next_delay(MIN_INTERVAL, Err(&long), 0).0,
            Duration::from_secs(7_200)
        );
        // Any other failure backs off too, and a success clears the count.
        let other = ReadError::Other("HTTP 500".into());
        assert_eq!(
            next_delay(MIN_INTERVAL, Err(&other), 0),
            (MIN_INTERVAL * 2, 1)
        );
        assert_eq!(next_delay(MIN_INTERVAL, Ok(()), 5).1, 0);
    }

    #[test]
    fn a_manual_refresh_cannot_hammer_a_provider() {
        let fresh = Slot {
            read_at: Some(100_000),
            ..Default::default()
        };
        assert!(!may_refresh(&fresh, 100_000 + MANUAL_FLOOR_MS - 1));
        assert!(may_refresh(&fresh, 100_000 + MANUAL_FLOOR_MS));
        assert!(may_refresh(&Slot::default(), 0));
        let limited = Slot {
            rate_limited: true,
            ..Default::default()
        };
        assert!(!may_refresh(&limited, i64::MAX));
    }

    #[test]
    fn the_app_ring_is_built_from_telemetry_alone() {
        use crate::llm_manager::{CostBucket, QuotaStatus, TelemetrySnapshot};
        let t = TelemetrySnapshot {
            quotas: vec![QuotaStatus {
                id: "acc1".into(),
                label: "Work".into(),
                kind: "cli".into(),
                used: 0.42,
                resets_at_ms: Some(1_000),
                source: "reported".into(),
                exhausts_at_ms: None,
                active: true,
            }],
            cost_by_day: vec![CostBucket {
                day: "2026-09-19".into(),
                cost_usd: 1.5,
                calls: 3,
            }],
            ..Default::default()
        };
        let r = omniget_reading(&t);
        assert_eq!(r.windows[0].used, Some(0.42));
        assert_eq!(r.windows[0].resets_at, Some(1_000));
        assert_eq!(r.windows[0].group, None);
        assert_eq!(r.note.as_deref(), Some("today: $1.50 in 3 calls"));
        let empty = omniget_reading(&TelemetrySnapshot::default());
        assert_eq!(empty.note.as_deref(), Some("no account yet"));
    }

    #[test]
    fn a_snapshot_lists_only_what_is_switched_on() {
        let inner = Inner {
            prefs: prefs(&[OMNIGET_ID, "ollama"], &["claude"]),
            readers: providers::all_with(&[]),
            ..Default::default()
        };
        let snap = build_snapshot(&inner, true, 0);
        let ids: Vec<_> = snap.rings.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, [OMNIGET_ID, "ollama"]);
        assert_eq!(snap.rings[1].status, "pending");
        assert!(snap.rings[1].local && !snap.rings[1].beta);
    }

    #[test]
    fn a_new_account_starts_like_its_cli_ring() {
        use omniget_core::core::llm::cli_runtime::accounts::{CliAccount, CliKind};
        let accounts = [CliAccount {
            id: "acc-1".into(),
            cli: CliKind::Claude,
            config_dir: "/p/acc-1".into(),
            label: "Max".into(),
            disabled: false,
            sandbox: Default::default(),
        }];
        let readers = providers::all_with(&accounts);
        let mut g = Inner {
            prefs: prefs(&["claude"], &["codex"]),
            ..Default::default()
        };
        assert!(take_readers(&mut g, readers.clone()));
        assert!(
            g.prefs.is_on("claude:acc-1"),
            "claude is on, so is the newcomer"
        );
        let k = known(&readers, &prefs(&[], &["claude"]));
        assert!(k.contains(&("claude:acc-1".to_string(), false)));
        assert!(k.contains(&("ollama".to_string(), true)));
        // The account is removed in the app: its ring and reading go.
        g.held.insert("claude:acc-1".into(), Held::default());
        assert!(take_readers(&mut g, providers::all_with(&[])));
        assert!(g.prefs.provider("claude:acc-1").is_none());
        assert!(!g.held.contains_key("claude:acc-1"));
        let snap = build_snapshot(&g, true, 0);
        assert!(snap.rings.iter().all(|r| r.id != "claude:acc-1"));
    }
}
