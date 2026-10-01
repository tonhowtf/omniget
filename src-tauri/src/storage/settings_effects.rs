//! What a confirmed settings change does to the running app, kept apart
//! from the persistence: saving and applying are two results.
//!
//! Each [`EffectId`] is a domain with a serial queue: at most one run in
//! flight, a target revision, and the private snapshot of that revision. A
//! newer commit replaces a target that has not started; a run that finishes
//! for an old revision never marks the newer one applied, the worker goes on
//! to the latest target first. No run happens under the repository lock, and
//! no effect talks to a provider, creates a grant or starts a download.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::models::settings::AppSettings;
use crate::storage::settings_repository::Commit;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum EffectId {
    #[serde(rename = "network.proxy")]
    NetworkProxy,
    #[serde(rename = "downloads.segments")]
    DownloadsSegments,
    #[serde(rename = "downloads.queue")]
    DownloadsQueue,
    #[serde(rename = "shortcuts.registration")]
    ShortcutsRegistration,
    #[serde(rename = "app.autostart")]
    AppAutostart,
    #[serde(rename = "integrations.rpc")]
    IntegrationsRpc,
    #[serde(rename = "llm.pruning")]
    LlmPruning,
    #[serde(rename = "llm.watchdog")]
    LlmWatchdog,
    #[serde(rename = "llm.jobs")]
    LlmJobs,
    #[serde(rename = "bridge.lifecycle")]
    BridgeLifecycle,
}

impl EffectId {
    pub const ALL: [EffectId; 10] = [
        Self::NetworkProxy,
        Self::DownloadsSegments,
        Self::DownloadsQueue,
        Self::ShortcutsRegistration,
        Self::AppAutostart,
        Self::IntegrationsRpc,
        Self::LlmPruning,
        Self::LlmWatchdog,
        Self::LlmJobs,
        Self::BridgeLifecycle,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::NetworkProxy => "network.proxy",
            Self::DownloadsSegments => "downloads.segments",
            Self::DownloadsQueue => "downloads.queue",
            Self::ShortcutsRegistration => "shortcuts.registration",
            Self::AppAutostart => "app.autostart",
            Self::IntegrationsRpc => "integrations.rpc",
            Self::LlmPruning => "llm.pruning",
            Self::LlmWatchdog => "llm.watchdog",
            Self::LlmJobs => "llm.jobs",
            Self::BridgeLifecycle => "bridge.lifecycle",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.as_str() == s)
    }

    /// The settings paths whose change calls for this effect.
    fn paths(self) -> &'static [&'static str] {
        match self {
            Self::NetworkProxy => &["proxy"],
            Self::DownloadsSegments => &["advanced.max_concurrent_segments"],
            Self::DownloadsQueue => &[
                "advanced.max_concurrent_downloads",
                "advanced.stagger_delay_ms",
                "advanced.max_retries",
                "advanced.prevent_sleep",
            ],
            Self::ShortcutsRegistration => &[
                "download.hotkey_enabled",
                "download.hotkey_binding",
                "download.music_hotkey_enabled",
                "download.music_hotkey_binding",
            ],
            Self::AppAutostart => &["start_with_system", "run_in_background"],
            Self::IntegrationsRpc => &["rpc"],
            Self::LlmPruning => &[
                "llm.prune_enabled",
                "llm.prune_judge",
                "llm.prune_min_tokens",
            ],
            Self::LlmWatchdog => &[
                "llm.turn_idle_timeout_s",
                "llm.turn_tool_idle_timeout_s",
                "llm.turn_start_timeout_s",
                "llm.job_default_deadline_min",
                "llm.ask_timeout_chat_s",
                "llm.ask_timeout_job_s",
                "llm.approval_notify_delay_s",
            ],
            Self::LlmJobs => &["llm.jobs_max_parallel", "llm.jobs_max_per_agent"],
            // The port is managed by the bridge itself (and protected in a
            // patch); only switching the bridge on or off is a user effect.
            Self::BridgeLifecycle => &["bridge.enabled"],
        }
    }
}

/// The effects whose fields differ between two settings.
pub fn effects_for_change(before: &AppSettings, after: &AppSettings) -> Vec<EffectId> {
    let (Ok(a), Ok(b)) = (serde_json::to_value(before), serde_json::to_value(after)) else {
        return Vec::new();
    };
    let get = crate::models::settings::get_path;
    EffectId::ALL
        .into_iter()
        .filter(|id| id.paths().iter().any(|p| get(&a, p) != get(&b, p)))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailCode {
    ApplyFailed,
    NotAvailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied,
    Failed(FailCode),
    /// Takes effect on the next start; never simulated.
    RestartRequired,
}

/// The local operations behind each effect. `previous` is the snapshot the
/// domain last applied (or the one before its first change), for effects
/// that need the old value (RPC app id).
pub trait EffectApplier: Send + Sync + 'static {
    fn apply(&self, id: EffectId, previous: &AppSettings, settings: &AppSettings) -> ApplyOutcome;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectState {
    Queued,
    Applying,
    Failed,
    RestartRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingEffect {
    pub id: EffectId,
    /// Decimal u64, like every revision on the IPC.
    pub target_revision: String,
    pub state: EffectState,
    pub code: Option<&'static str>,
    pub retryable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryOutcome {
    Applied,
    Pending,
    Superseded,
}

#[derive(Default)]
struct Domain {
    /// Latest wanted revision and its snapshot.
    target: Option<(u64, Arc<AppSettings>)>,
    /// Run the target even if it was applied (explicit retry).
    force: bool,
    running: Option<u64>,
    applied: Option<u64>,
    failure: Option<(u64, FailCode)>,
    restart: Option<u64>,
    /// What the domain last applied, or the state before its first change.
    baseline: Option<Arc<AppSettings>>,
    worker: bool,
}

pub struct EffectsRegistry {
    applier: Arc<dyn EffectApplier>,
    notify: Arc<dyn Fn() + Send + Sync>,
    domains: Mutex<HashMap<EffectId, Domain>>,
    settled: Condvar,
}

impl EffectsRegistry {
    pub fn new(applier: Arc<dyn EffectApplier>, notify: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            applier,
            notify,
            domains: Mutex::new(HashMap::new()),
            settled: Condvar::new(),
        })
    }

    fn guard(&self) -> MutexGuard<'_, HashMap<EffectId, Domain>> {
        self.domains.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Records the baseline of domains that have none yet (boot state), so
    /// the first change knows what it replaces.
    pub fn set_baseline(&self, settings: &AppSettings) {
        let snapshot = Arc::new(settings.clone());
        let mut domains = self.guard();
        for id in EffectId::ALL {
            let d = domains.entry(id).or_default();
            if d.baseline.is_none() {
                d.baseline = Some(snapshot.clone());
            }
        }
    }

    /// Queues `ids` for `revision`. Newer targets replace older ones that
    /// have not started.
    pub fn schedule(
        self: &Arc<Self>,
        ids: &[EffectId],
        revision: u64,
        before: &AppSettings,
        after: &AppSettings,
    ) {
        if ids.is_empty() {
            return;
        }
        let snapshot = Arc::new(after.clone());
        let mut spawn = Vec::new();
        {
            let mut domains = self.guard();
            for &id in ids {
                let d = domains.entry(id).or_default();
                if d.baseline.is_none() {
                    d.baseline = Some(Arc::new(before.clone()));
                }
                if d.target.as_ref().is_none_or(|(r, _)| *r <= revision) {
                    d.target = Some((revision, snapshot.clone()));
                }
                if !d.worker {
                    d.worker = true;
                    spawn.push(id);
                }
            }
        }
        for id in spawn {
            let me = self.clone();
            std::thread::Builder::new()
                .name(format!("settings-effect-{}", id.as_str()))
                .spawn(move || me.run(id))
                .ok();
        }
    }

    fn run(&self, id: EffectId) {
        loop {
            let (revision, settings, previous) = {
                let mut domains = self.guard();
                let d = domains.entry(id).or_default();
                let Some((revision, settings)) = d.target.clone() else {
                    d.worker = false;
                    break;
                };
                let done = d.applied == Some(revision)
                    || d.failure.is_some_and(|(r, _)| r == revision)
                    || d.restart == Some(revision);
                if done && !d.force {
                    d.worker = false;
                    break;
                }
                d.force = false;
                d.running = Some(revision);
                let previous = d.baseline.clone().unwrap_or_else(|| settings.clone());
                (revision, settings, previous)
            };
            self.settled.notify_all();
            let outcome = self.applier.apply(id, &previous, &settings);
            {
                let mut domains = self.guard();
                let d = domains.entry(id).or_default();
                d.running = None;
                match outcome {
                    ApplyOutcome::Applied => {
                        d.applied = Some(revision);
                        d.failure = None;
                        d.restart = None;
                        d.baseline = Some(settings.clone());
                    }
                    ApplyOutcome::Failed(code) => d.failure = Some((revision, code)),
                    ApplyOutcome::RestartRequired => {
                        d.failure = None;
                        d.restart = Some(revision);
                    }
                }
            }
            self.settled.notify_all();
            (self.notify)();
        }
        self.settled.notify_all();
    }

    fn pending_of(id: EffectId, d: &Domain) -> Option<PendingEffect> {
        let (target, _) = d.target.as_ref()?;
        let target = *target;
        if d.applied == Some(target) && d.running.is_none() {
            return None;
        }
        let (state, code, retryable) = if d.running.is_some() {
            (EffectState::Applying, None, false)
        } else if d.restart == Some(target) {
            (
                EffectState::RestartRequired,
                Some("restart_required"),
                false,
            )
        } else if let Some((_, code)) = d.failure.filter(|(r, _)| *r == target) {
            let code = match code {
                FailCode::ApplyFailed => "apply_failed",
                FailCode::NotAvailable => "not_available",
            };
            (EffectState::Failed, Some(code), true)
        } else {
            (EffectState::Queued, None, false)
        };
        Some(PendingEffect {
            id,
            target_revision: target.to_string(),
            state,
            code,
            retryable,
        })
    }

    /// Every effect not applied at its latest target, in a stable order.
    pub fn pending(&self) -> Vec<PendingEffect> {
        let domains = self.guard();
        EffectId::ALL
            .into_iter()
            .filter_map(|id| domains.get(&id).and_then(|d| Self::pending_of(id, d)))
            .collect()
    }

    /// Waits until none of `ids` is queued or running, up to `timeout`.
    pub fn wait_idle(&self, ids: &[EffectId], timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut domains = self.guard();
        loop {
            let busy = ids.iter().any(|id| {
                domains.get(id).is_some_and(|d| {
                    d.running.is_some()
                        || matches!(
                            Self::pending_of(*id, d).map(|p| p.state),
                            Some(EffectState::Queued)
                        )
                })
            });
            let now = Instant::now();
            if !busy || now >= deadline {
                return;
            }
            domains = self
                .settled
                .wait_timeout(domains, deadline - now)
                .map(|(g, _)| g)
                .unwrap_or_else(|p| p.into_inner().0);
        }
    }

    /// `settings_retry_effect_v3`: reapplies the latest intention of `id`
    /// with the latest confirmed settings, never an old one.
    /// `current_revision`/`current` = the repository's confirmed state;
    /// `target_revision` must already be checked `<= current_revision`.
    pub fn retry(
        self: &Arc<Self>,
        id: EffectId,
        target_revision: u64,
        current_revision: u64,
        current: &AppSettings,
        wait: Duration,
    ) -> RetryOutcome {
        let (latest, pending) = {
            let domains = self.guard();
            match domains.get(&id) {
                Some(d) => (d.target.as_ref().map(|(r, _)| *r), Self::pending_of(id, d)),
                None => (None, None),
            }
        };
        let latest = latest.unwrap_or(current_revision);
        if target_revision < latest {
            // Old target: nothing old is applied. A still pending latest
            // intention is queued again with the current snapshot.
            if pending
                .as_ref()
                .is_some_and(|p| p.state == EffectState::Failed)
            {
                self.force(id, current_revision, current);
            }
            return RetryOutcome::Superseded;
        }
        let has_target = self.guard().get(&id).is_some_and(|d| d.target.is_some());
        match pending.map(|p| p.state) {
            Some(EffectState::RestartRequired) => return RetryOutcome::Pending,
            // Applied at this target: nothing to repeat.
            None if has_target => return RetryOutcome::Applied,
            _ => {}
        }
        self.force(id, current_revision, current);
        self.wait_idle(&[id], wait);
        let domains = self.guard();
        match domains.get(&id).and_then(|d| Self::pending_of(id, d)) {
            None => RetryOutcome::Applied,
            Some(_) => RetryOutcome::Pending,
        }
    }

    fn force(self: &Arc<Self>, id: EffectId, revision: u64, current: &AppSettings) {
        {
            let mut domains = self.guard();
            let d = domains.entry(id).or_default();
            d.force = true;
            d.failure = None;
            d.restart = None;
            d.target = Some((revision, Arc::new(current.clone())));
            if d.worker {
                return;
            }
            d.worker = true;
        }
        let me = self.clone();
        std::thread::Builder::new()
            .name(format!("settings-effect-{}", id.as_str()))
            .spawn(move || me.run(id))
            .ok();
    }
}

// ---------------------------------------------------------------------------
// The desktop registry
// ---------------------------------------------------------------------------

static REGISTRY: OnceLock<Arc<EffectsRegistry>> = OnceLock::new();

pub fn registry() -> Option<Arc<EffectsRegistry>> {
    REGISTRY.get().cloned()
}

/// Wires the effects to the running app. Called once from `setup`, after
/// the boot code applied the loaded settings.
pub fn install(app: tauri::AppHandle) {
    let boot = crate::storage::settings_repository::global().settings();
    let notify_app = app.clone();
    let registry = EffectsRegistry::new(
        Arc::new(DesktopApplier {
            app,
            bridge_enabled_at_boot: boot.bridge.enabled,
        }),
        Arc::new(move || emit_changed(&notify_app)),
    );
    registry.set_baseline(&boot);
    let _ = REGISTRY.set(registry);
}

/// `settings-v3-changed`: the revision only, never settings or secrets.
pub fn emit_changed(app: &tauri::AppHandle) {
    use tauri::Emitter;
    let revision = crate::storage::settings_repository::global().revision();
    let _ = app.emit(
        "settings-v3-changed",
        serde_json::json!({ "revision": revision.to_string() }),
    );
}

/// After a confirmed commit (lock released): schedule the effects of the
/// fields that changed and tell the UI about the new revision.
pub fn after_commit(app: &tauri::AppHandle, commit: &Commit) {
    if !commit.changed {
        return;
    }
    if let Some(registry) = registry() {
        let ids = effects_for_change(&commit.before, &commit.after);
        registry.schedule(&ids, commit.revision, &commit.before, &commit.after);
    }
    crate::commands::settings::apply_live(app, &commit.before, &commit.after);
    emit_changed(app);
}

/// Lets a dedicated command (prune, watchdog, jobs) answer after its own
/// effect ran, as it did when it installed the change itself.
pub fn settle(ids: &[EffectId]) {
    if let Some(registry) = registry() {
        registry.wait_idle(ids, Duration::from_secs(5));
    }
}

struct DesktopApplier {
    app: tauri::AppHandle,
    bridge_enabled_at_boot: bool,
}

impl EffectApplier for DesktopApplier {
    fn apply(&self, id: EffectId, previous: &AppSettings, s: &AppSettings) -> ApplyOutcome {
        match id {
            EffectId::NetworkProxy => {
                crate::core::http_client::init_proxy(s.proxy.clone());
                ApplyOutcome::Applied
            }
            EffectId::DownloadsSegments => {
                crate::core::http_fetcher::set_global_max_concurrent_segments(
                    s.advanced.max_concurrent_segments as usize,
                );
                ApplyOutcome::Applied
            }
            EffectId::DownloadsQueue => {
                crate::commands::downloads::apply_queue_policy(&self.app, s);
                ApplyOutcome::Applied
            }
            EffectId::ShortcutsRegistration => match crate::hotkey::apply_bindings(&self.app, s) {
                Ok(()) => ApplyOutcome::Applied,
                Err(_) => ApplyOutcome::Failed(FailCode::ApplyFailed),
            },
            EffectId::AppAutostart => {
                match crate::commands::autostart::apply_autostart(
                    &self.app,
                    crate::commands::settings::autostart_wanted(s),
                ) {
                    Ok(()) => ApplyOutcome::Applied,
                    Err(_) => ApplyOutcome::Failed(FailCode::ApplyFailed),
                }
            }
            EffectId::IntegrationsRpc => {
                let next = s.rpc.clone();
                let prev_app_id = previous.rpc.app_id.clone();
                tauri::async_runtime::block_on(crate::core::rpc::handle_settings_changed(
                    next,
                    prev_app_id,
                ));
                ApplyOutcome::Applied
            }
            EffectId::LlmPruning => {
                crate::commands::llm::prune::install_with(&self.app, &s.llm);
                ApplyOutcome::Applied
            }
            EffectId::LlmWatchdog => {
                crate::commands::llm::watchdog::install_with(&self.app, &s.llm);
                ApplyOutcome::Applied
            }
            EffectId::LlmJobs => {
                // Without the jobs runtime the gate reads the settings when
                // it starts; there is nothing running to update.
                if let Some(jobs) = crate::jobs::try_get() {
                    jobs.apply_limits(&s.llm);
                }
                ApplyOutcome::Applied
            }
            EffectId::BridgeLifecycle => {
                // The server is started once at boot; switching it on or
                // off takes effect on the next start.
                if s.bridge.enabled == self.bridge_enabled_at_boot {
                    ApplyOutcome::Applied
                } else {
                    ApplyOutcome::RestartRequired
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    /// Records every run; blocks runs of revision 1 until released; fails
    /// while `fail` is set.
    struct Recorder {
        runs: Mutex<Vec<(EffectId, u64, String)>>,
        gate: Mutex<Option<mpsc::Receiver<()>>>,
        started: Mutex<Option<mpsc::Sender<()>>>,
        fail: std::sync::atomic::AtomicBool,
        calls: AtomicUsize,
    }

    impl Recorder {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                runs: Mutex::new(Vec::new()),
                gate: Mutex::new(None),
                started: Mutex::new(None),
                fail: std::sync::atomic::AtomicBool::new(false),
                calls: AtomicUsize::new(0),
            })
        }
    }

    impl EffectApplier for Recorder {
        fn apply(&self, id: EffectId, _prev: &AppSettings, s: &AppSettings) -> ApplyOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(tx) = self.started.lock().unwrap().as_ref() {
                let _ = tx.send(());
            }
            let rev: u64 = s.advanced.max_retries.into();
            if rev == 1 {
                if let Some(rx) = self.gate.lock().unwrap().as_ref() {
                    let _ = rx.recv();
                }
            }
            self.runs
                .lock()
                .unwrap()
                .push((id, rev, s.download.video_quality.clone()));
            if self.fail.load(Ordering::SeqCst) {
                ApplyOutcome::Failed(FailCode::ApplyFailed)
            } else {
                ApplyOutcome::Applied
            }
        }
    }

    /// Settings tagged with their revision (in `max_retries`) and a quality.
    fn at(rev: u64, quality: &str) -> AppSettings {
        let mut s = AppSettings::default();
        s.advanced.max_retries = rev as u32;
        s.download.video_quality = quality.into();
        s
    }

    fn registry(applier: Arc<Recorder>) -> Arc<EffectsRegistry> {
        EffectsRegistry::new(applier, Arc::new(|| {}))
    }

    #[test]
    fn only_the_changed_domains_are_scheduled() {
        let a = AppSettings::default();
        let mut b = a.clone();
        b.proxy.enabled = true;
        b.llm.jobs_max_parallel = 8;
        b.download.video_quality = "1080p".into();
        b.bridge.port = 1234;
        assert_eq!(
            effects_for_change(&a, &b),
            vec![EffectId::NetworkProxy, EffectId::LlmJobs]
        );
        assert!(effects_for_change(&a, &a).is_empty());
        for id in EffectId::ALL {
            assert_eq!(EffectId::parse(id.as_str()), Some(id));
            assert_eq!(
                serde_json::to_value(id).unwrap(),
                serde_json::json!(id.as_str())
            );
        }
        assert_eq!(EffectId::parse("network"), None);
    }

    // API-06: a slow run for revision 1 never marks revision 2 applied; the
    // worker reconciles to 2 right after, and the old target is not rerun.
    #[test]
    fn a_slow_old_run_does_not_mark_the_new_revision_applied() {
        let rec = Recorder::new();
        let (release, gate) = mpsc::channel();
        let (started_tx, started) = mpsc::channel();
        *rec.gate.lock().unwrap() = Some(gate);
        *rec.started.lock().unwrap() = Some(started_tx);
        let reg = registry(rec.clone());
        let id = EffectId::NetworkProxy;
        reg.schedule(&[id], 1, &AppSettings::default(), &at(1, "480p"));
        started.recv().unwrap();
        // Commit 2 arrives while 1 is still applying.
        reg.schedule(&[id], 2, &at(1, "480p"), &at(2, "1080p"));
        let p = reg.pending();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].target_revision, "2");
        assert_eq!(p[0].state, EffectState::Applying);
        release.send(()).unwrap();
        reg.wait_idle(&[id], Duration::from_secs(5));
        assert!(reg.pending().is_empty());
        let runs = rec.runs.lock().unwrap().clone();
        assert_eq!(runs, vec![(id, 1, "480p".into()), (id, 2, "1080p".into())]);
    }

    #[test]
    fn targets_not_started_are_replaced_by_the_newest() {
        let rec = Recorder::new();
        let (release, gate) = mpsc::channel();
        let (started_tx, started) = mpsc::channel();
        *rec.gate.lock().unwrap() = Some(gate);
        *rec.started.lock().unwrap() = Some(started_tx);
        let reg = registry(rec.clone());
        let id = EffectId::DownloadsQueue;
        reg.schedule(&[id], 1, &AppSettings::default(), &at(1, "a"));
        started.recv().unwrap();
        reg.schedule(&[id], 2, &at(1, "a"), &at(2, "b"));
        reg.schedule(&[id], 3, &at(2, "b"), &at(3, "c"));
        release.send(()).unwrap();
        reg.wait_idle(&[id], Duration::from_secs(5));
        let revs: Vec<u64> = rec.runs.lock().unwrap().iter().map(|r| r.1).collect();
        assert_eq!(revs, vec![1, 3]);
    }

    // API-06: independent domains do not wait for each other.
    #[test]
    fn a_slow_domain_does_not_block_another() {
        let rec = Recorder::new();
        let (release, gate) = mpsc::channel();
        *rec.gate.lock().unwrap() = Some(gate);
        let reg = registry(rec.clone());
        reg.schedule(
            &[EffectId::LlmWatchdog],
            1,
            &AppSettings::default(),
            &at(1, "slow"),
        );
        reg.schedule(
            &[EffectId::AppAutostart],
            2,
            &AppSettings::default(),
            &at(2, "fast"),
        );
        reg.wait_idle(&[EffectId::AppAutostart], Duration::from_secs(5));
        let p = reg.pending();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].id, EffectId::LlmWatchdog);
        release.send(()).unwrap();
        reg.wait_idle(&[EffectId::LlmWatchdog], Duration::from_secs(5));
        assert!(reg.pending().is_empty());
    }

    #[test]
    fn a_failure_is_pending_and_retryable_and_the_retry_reapplies_the_latest() {
        let rec = Recorder::new();
        rec.fail.store(true, Ordering::SeqCst);
        let reg = registry(rec.clone());
        let id = EffectId::ShortcutsRegistration;
        reg.schedule(&[id], 4, &AppSettings::default(), &at(4, "x"));
        reg.wait_idle(&[id], Duration::from_secs(5));
        let p = reg.pending();
        assert_eq!(p[0].state, EffectState::Failed);
        assert_eq!(p[0].code, Some("apply_failed"));
        assert!(p[0].retryable);
        // Not retried on its own.
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(rec.calls.load(Ordering::SeqCst), 1);

        rec.fail.store(false, Ordering::SeqCst);
        // An old target is superseded: revision 3's value is never applied,
        // the latest intention (6, current snapshot) is queued instead.
        let current = at(6, "latest");
        assert_eq!(
            reg.retry(id, 3, 6, &current, Duration::from_secs(5)),
            RetryOutcome::Superseded
        );
        reg.wait_idle(&[id], Duration::from_secs(5));
        let runs = rec.runs.lock().unwrap().clone();
        assert_eq!(runs.last().unwrap().2, "latest");
        assert!(runs.iter().all(|r| r.1 != 3));
        assert!(reg.pending().is_empty());

        // Retry at the applied target: nothing repeated.
        let before = rec.calls.load(Ordering::SeqCst);
        assert_eq!(
            reg.retry(id, 6, 6, &current, Duration::from_secs(5)),
            RetryOutcome::Applied
        );
        assert_eq!(rec.calls.load(Ordering::SeqCst), before);
    }

    #[test]
    fn a_retry_reapplies_even_without_a_value_change() {
        let rec = Recorder::new();
        rec.fail.store(true, Ordering::SeqCst);
        let reg = registry(rec.clone());
        let id = EffectId::AppAutostart;
        let s = at(2, "same");
        reg.schedule(&[id], 2, &s, &s);
        reg.wait_idle(&[id], Duration::from_secs(5));
        rec.fail.store(false, Ordering::SeqCst);
        assert_eq!(
            reg.retry(id, 2, 2, &s, Duration::from_secs(5)),
            RetryOutcome::Applied
        );
        assert_eq!(rec.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn restart_required_stays_pending_and_is_not_simulated() {
        struct Restart;
        impl EffectApplier for Restart {
            fn apply(&self, _: EffectId, _: &AppSettings, _: &AppSettings) -> ApplyOutcome {
                ApplyOutcome::RestartRequired
            }
        }
        let reg = EffectsRegistry::new(Arc::new(Restart), Arc::new(|| {}));
        let id = EffectId::BridgeLifecycle;
        let s = at(1, "x");
        reg.schedule(&[id], 1, &s, &s);
        reg.wait_idle(&[id], Duration::from_secs(5));
        let p = reg.pending();
        assert_eq!(p[0].state, EffectState::RestartRequired);
        assert!(!p[0].retryable);
        assert_eq!(
            reg.retry(id, 1, 1, &s, Duration::from_secs(1)),
            RetryOutcome::Pending
        );
    }
}
