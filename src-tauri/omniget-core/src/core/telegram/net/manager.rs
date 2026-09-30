//! Connection manager: owns the routes, tests them, picks the best one and
//! switches on failure. The MTProto client never sees any of this; it talks
//! to the bridge, which asks the manager for routes.
//!
//! Automatic mode (the default) is what makes it work with zero setup:
//! on start it gathers every candidate (direct, the system proxy, every
//! local proxy app it can find, saved and subscription routes, Telegram's
//! own rescue proxies), probes them all in parallel with a real MTProto
//! round trip and keeps the fastest working one, remembering it for next
//! time. When the active route fails twice in a row it re-scans in the
//! background, like Telegram Desktop's auto-switch.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use super::bridge::{self, Bridge, RouteProvider};
use super::detect::{self, Detected};
use super::route::{Route, RouteKind, RouteSource};
use super::{clock, mihomo, probe, rescue, subscription, NetOpts};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Auto,
    Fixed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionEntry {
    pub url: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub count: usize,
    #[serde(default)]
    pub unsupported: Vec<(String, usize)>,
}

/// What we persist in `<data>/telegram/network.json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetConfig {
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub fixed_route: Option<String>,
    /// Routes the person added (links, manual, subscriptions).
    #[serde(default)]
    pub routes: Vec<Route>,
    #[serde(default)]
    pub subscriptions: Vec<SubscriptionEntry>,
    #[serde(default)]
    pub opts: NetOpts,
    /// Try Telegram's own rescue proxies (from the DNS "simple config").
    #[serde(default = "yes")]
    pub use_rescue: bool,
    /// Last route that worked, tried first on the next start.
    #[serde(default)]
    pub last_good: Option<String>,
    /// Run the mihomo sidecar for VLESS/Hysteria2/TUIC/… nodes. `None`
    /// means never asked: the screen offers it once such nodes exist.
    #[serde(default)]
    pub use_mihomo: Option<bool>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Health {
    pub ok: Option<bool>,
    pub rtt_ms: Option<u64>,
    pub connect_ms: Option<u64>,
    pub error: Option<String>,
    pub tested_at: i64,
    pub fails_in_row: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteView {
    pub route: Route,
    pub health: Health,
    pub saved: bool,
    pub active: bool,
    pub share_link: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Scanning,
    Connected,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub phase: Phase,
    pub mode: Mode,
    pub active: Option<RouteView>,
    pub routes: Vec<RouteView>,
    pub scanned_at: i64,
    pub clock_offset_secs: Option<i64>,
    pub opts: NetOpts,
    pub use_rescue: bool,
    pub subscriptions: Vec<SubscriptionEntry>,
    pub bridge_port: Option<u16>,
    pub mihomo: mihomo::MihomoStatus,
}

pub struct Manager {
    cfg: Mutex<NetConfig>,
    detected: Mutex<Vec<Detected>>,
    rescue: Mutex<Vec<Route>>,
    health: Mutex<HashMap<String, Health>>,
    active: Mutex<Option<String>>,
    scanning: AtomicBool,
    scanned_at: Mutex<i64>,
    active_fails: AtomicU32,
    bridge: tokio::sync::OnceCell<Bridge>,
    scan_done: Notify,
    path: PathBuf,
}

static MANAGER: OnceLock<Arc<Manager>> = OnceLock::new();

/// The process-wide manager, loaded from disk on first use.
pub fn get() -> Arc<Manager> {
    MANAGER
        .get_or_init(|| {
            let dir = crate::core::paths::app_data_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("telegram");
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("network.json");
            let cfg: NetConfig = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_else(|| NetConfig {
                    use_rescue: true,
                    ..Default::default()
                });
            Arc::new(Manager {
                active: Mutex::new(cfg.last_good.clone()),
                cfg: Mutex::new(cfg),
                detected: Mutex::new(Vec::new()),
                rescue: Mutex::new(Vec::new()),
                health: Mutex::new(HashMap::new()),
                scanning: AtomicBool::new(false),
                scanned_at: Mutex::new(0),
                active_fails: AtomicU32::new(0),
                bridge: tokio::sync::OnceCell::new(),
                scan_done: Notify::new(),
                path,
            })
        })
        .clone()
}

struct Provider(Arc<Manager>);

impl RouteProvider for Provider {
    fn routes(&self) -> Vec<Route> {
        self.0.ordered_routes()
    }
    fn opts(&self) -> NetOpts {
        self.0.cfg.lock().unwrap().opts.clone()
    }
    fn report(&self, route_id: &str, ok: bool, error: Option<String>) {
        self.0.report(route_id, ok, error);
    }
}

impl Manager {
    /// The loopback SOCKS5 URL for the MTProto client. Starts the bridge on
    /// first call.
    pub async fn bridge_url(self: &Arc<Self>) -> anyhow::Result<String> {
        let me = self.clone();
        let b = self
            .bridge
            .get_or_try_init(|| async move { bridge::start(Arc::new(Provider(me))).await })
            .await?;
        Ok(b.url())
    }

    fn save(&self) {
        let cfg = self.cfg.lock().unwrap().clone();
        if let Ok(text) = serde_json::to_string_pretty(&cfg) {
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }

    /// Every route we know, deduplicated, saved ones first.
    fn all_routes(&self) -> Vec<(Route, bool)> {
        let cfg = self.cfg.lock().unwrap();
        let mut out: Vec<(Route, bool)> = vec![(Route::direct(), false)];
        for r in &cfg.routes {
            out.push((r.clone(), true));
        }
        for d in self.detected.lock().unwrap().iter() {
            out.push((d.route.clone(), false));
        }
        if cfg.use_rescue {
            for r in self.rescue.lock().unwrap().iter() {
                out.push((r.clone(), false));
            }
        }
        // Listeners of the mihomo sidecar: one SOCKS5 route per node, only
        // while it runs. They count as saved (the node is).
        for r in mihomo::routes() {
            out.push((r, true));
        }
        let mut seen = std::collections::HashSet::new();
        out.retain(|(r, _)| seen.insert(r.id.clone()));
        out
    }

    fn find(&self, id: &str) -> Option<Route> {
        self.all_routes()
            .into_iter()
            .map(|(r, _)| r)
            .find(|r| r.id == id)
    }

    /// Order in which the bridge tries routes for a new connection.
    pub fn ordered_routes(&self) -> Vec<Route> {
        let (mode, fixed) = {
            let cfg = self.cfg.lock().unwrap();
            (cfg.mode.clone(), cfg.fixed_route.clone())
        };
        let mut pinned = None;
        if mode == Mode::Fixed {
            if let Some(r) = fixed.and_then(|id| self.find(&id)) {
                // A per-DC endpoint cannot carry the other DCs: those fall
                // through to the automatic order behind it.
                if r.dc_only.is_none() {
                    return vec![r];
                }
                pinned = Some(r);
            }
        }
        let health = self.health.lock().unwrap().clone();
        let active = self.active.lock().unwrap().clone();
        let mut routes: Vec<Route> = self.all_routes().into_iter().map(|(r, _)| r).collect();
        // Public/rescue proxies are shared and slow under load: they are the
        // fallback, used only while nothing of the person's own works.
        let score = |r: &Route| -> (u8, u8, u64) {
            let h = health.get(&r.id);
            let public = matches!(r.source, RouteSource::PublicList { .. }) as u8;
            if active.as_deref() == Some(r.id.as_str())
                && h.map(|h| h.fails_in_row < 2).unwrap_or(true)
            {
                return (0, 0, 0);
            }
            match h.and_then(|h| h.ok) {
                Some(true) => (1, public, h.and_then(|h| h.rtt_ms).unwrap_or(9999)),
                None => (2, public, source_rank(&r.source)),
                Some(false) => (3, public, h.map(|h| h.fails_in_row as u64).unwrap_or(0)),
            }
        };
        routes.sort_by_key(|r| score(r));
        // Known-bad routes only cost time; keep them out while anything
        // better exists, but never hand back an empty list.
        let good = routes
            .iter()
            .filter(|r| health.get(&r.id).and_then(|h| h.ok) != Some(false))
            .count();
        if good > 0 {
            routes.retain(|r| health.get(&r.id).and_then(|h| h.ok) != Some(false));
        }
        routes.truncate(6);
        if let Some(p) = pinned {
            routes.retain(|r| r.id != p.id);
            routes.insert(0, p);
        }
        routes
    }

    fn report(self: &Arc<Self>, route_id: &str, ok: bool, error: Option<String>) {
        {
            let mut h = self.health.lock().unwrap();
            let e = h.entry(route_id.to_string()).or_default();
            e.tested_at = clock::system_unix();
            if ok {
                e.ok = Some(true);
                e.fails_in_row = 0;
                e.error = None;
            } else {
                e.fails_in_row += 1;
                e.error = error;
                if e.fails_in_row >= 2 {
                    e.ok = Some(false);
                }
            }
        }
        let is_active = self.active.lock().unwrap().as_deref() == Some(route_id);
        if ok {
            *self.active.lock().unwrap() = Some(route_id.to_string());
            self.active_fails.store(0, Ordering::Relaxed);
            let changed = {
                let mut cfg = self.cfg.lock().unwrap();
                let changed = cfg.last_good.as_deref() != Some(route_id);
                cfg.last_good = Some(route_id.to_string());
                changed
            };
            if changed {
                self.save();
            }
        } else if is_active && self.active_fails.fetch_add(1, Ordering::Relaxed) + 1 >= 2 {
            self.active_fails.store(0, Ordering::Relaxed);
            let me = self.clone();
            tokio::spawn(async move {
                me.scan().await;
            });
        }
    }

    /// Finds candidates and probes them all. Safe to call concurrently:
    /// a second caller waits for the running scan.
    pub async fn scan(self: &Arc<Self>) -> Status {
        if self.scanning.swap(true, Ordering::SeqCst) {
            self.scan_done.notified().await;
            return self.status();
        }
        let detected = detect::detect_all().await;
        *self.detected.lock().unwrap() = detected;
        if self.cfg.lock().unwrap().use_rescue && self.rescue.lock().unwrap().is_empty() {
            match tokio::time::timeout(std::time::Duration::from_secs(8), rescue::fetch()).await {
                Ok(Ok(eps)) => *self.rescue.lock().unwrap() = rescue::routes(&eps),
                Ok(Err(e)) => tracing::debug!("[tg-net] rescue config unavailable: {e:#}"),
                Err(_) => tracing::debug!("[tg-net] rescue config timed out"),
            }
        }
        self.mihomo_ensure(false).await;
        let routes: Vec<Route> = self.all_routes().into_iter().map(|(r, _)| r).collect();
        self.probe_many(routes).await;

        let best = {
            let h = self.health.lock().unwrap();
            let public: std::collections::HashSet<String> = self
                .rescue
                .lock()
                .unwrap()
                .iter()
                .map(|r| r.id.clone())
                .collect();
            let mut ok: Vec<(String, bool, u64)> = h
                .iter()
                .filter(|(_, v)| v.ok == Some(true))
                .map(|(k, v)| (k.clone(), public.contains(k), v.rtt_ms.unwrap_or(9999)))
                .collect();
            ok.sort_by_key(|(_, public, rtt)| (*public, *rtt));
            ok.first().map(|(k, _, _)| k.clone())
        };
        if let Some(id) = &best {
            *self.active.lock().unwrap() = Some(id.clone());
            self.cfg.lock().unwrap().last_good = Some(id.clone());
            self.save();
        }
        *self.scanned_at.lock().unwrap() = clock::system_unix();
        self.scanning.store(false, Ordering::SeqCst);
        self.scan_done.notify_waiters();
        self.status()
    }

    /// Probes routes in parallel, eight at a time, and records the result.
    pub async fn probe_many(self: &Arc<Self>, routes: Vec<Route>) {
        use futures::StreamExt;
        let opts = self.cfg.lock().unwrap().opts.clone();
        let results: Vec<(String, anyhow::Result<probe::ProbeOk>)> = futures::stream::iter(routes)
            .map(|r| {
                let opts = opts.clone();
                async move {
                    let res = probe::probe(&r, r.dc_only.unwrap_or(2), &opts).await;
                    (r.id.clone(), res)
                }
            })
            .buffer_unordered(8)
            .collect()
            .await;
        let mut h = self.health.lock().unwrap();
        for (id, res) in results {
            let e = h.entry(id).or_default();
            e.tested_at = clock::system_unix();
            match res {
                Ok(p) => {
                    e.ok = Some(true);
                    e.rtt_ms = Some(p.rtt_ms);
                    e.connect_ms = Some(p.connect_ms);
                    e.error = None;
                    e.fails_in_row = 0;
                }
                Err(err) => {
                    e.ok = Some(false);
                    e.rtt_ms = None;
                    e.error = Some(format!("{err:#}"));
                    e.fails_in_row += 1;
                }
            }
        }
    }

    pub async fn test_route(self: &Arc<Self>, id: &str) -> anyhow::Result<RouteView> {
        let r = self
            .find(id)
            .ok_or_else(|| anyhow::anyhow!("unknown route"))?;
        self.probe_many(vec![r]).await;
        self.view(id)
            .ok_or_else(|| anyhow::anyhow!("unknown route"))
    }

    fn view(&self, id: &str) -> Option<RouteView> {
        let active = self.active.lock().unwrap().clone();
        let health = self.health.lock().unwrap();
        self.all_routes()
            .into_iter()
            .find(|(r, _)| r.id == id)
            .map(|(r, saved)| RouteView {
                health: health.get(&r.id).cloned().unwrap_or_default(),
                active: active.as_deref() == Some(r.id.as_str()),
                share_link: r.share_link(),
                route: r,
                saved,
            })
    }

    pub fn status(&self) -> Status {
        let active_id = self.active.lock().unwrap().clone();
        let health = self.health.lock().unwrap().clone();
        let routes: Vec<RouteView> = self
            .all_routes()
            .into_iter()
            .map(|(r, saved)| RouteView {
                health: health.get(&r.id).cloned().unwrap_or_default(),
                active: active_id.as_deref() == Some(r.id.as_str()),
                share_link: r.share_link(),
                route: r,
                saved,
            })
            .collect();
        let active = routes.iter().find(|v| v.active).cloned();
        let scanning = self.scanning.load(Ordering::Relaxed);
        let phase = if scanning {
            Phase::Scanning
        } else if active.as_ref().and_then(|a| a.health.ok) == Some(true) {
            Phase::Connected
        } else if routes.iter().any(|v| v.health.ok.is_some()) {
            if routes.iter().any(|v| v.health.ok == Some(true)) {
                Phase::Connected
            } else {
                Phase::Failed
            }
        } else {
            Phase::Idle
        };
        let cfg = self.cfg.lock().unwrap();
        Status {
            phase,
            mode: cfg.mode.clone(),
            active,
            routes,
            scanned_at: *self.scanned_at.lock().unwrap(),
            clock_offset_secs: clock::offset(),
            opts: cfg.opts.clone(),
            use_rescue: cfg.use_rescue,
            subscriptions: cfg.subscriptions.clone(),
            bridge_port: self.bridge.get().map(|b| b.port),
            mihomo: mihomo::status(cfg.use_mihomo),
        }
    }

    /// Adds routes parsed from pasted text. Returns how many were new and
    /// the protocols we could not use.
    ///
    /// Links only mihomo can dial are stored as mihomo nodes; while the
    /// sidecar is off their schemes are still reported as unsupported so
    /// the screen can offer to turn it on (see `Status::mihomo`). When it
    /// is on, it is (re)loaded in the background and the new nodes probed.
    pub fn add_from_text(self: &Arc<Self>, text: &str) -> (Vec<Route>, Vec<String>) {
        let parsed = super::links::parse_many(text, RouteSource::Link);
        let nodes = mihomo::extract_nodes(text, RouteSource::Link);
        // An `ss://` with a plugin also parses natively (the plugin is
        // dropped) and would never work: mihomo owns it.
        let taken: std::collections::HashSet<(String, u16)> = nodes
            .iter()
            .filter(|n| n.kind == "ss")
            .filter_map(|n| {
                let host = n.proxy.get("server")?.as_str()?.to_ascii_lowercase();
                let port = n.proxy.get("port")?.as_u64()? as u16;
                Some((host, port))
            })
            .collect();
        let mihomo_on = self.cfg.lock().unwrap().use_mihomo == Some(true);
        let mihomo_kinds: std::collections::HashSet<String> =
            nodes.iter().map(|n| n.kind.clone()).collect();
        let mut added = Vec::new();
        let mut unsupported = Vec::new();
        {
            let mut cfg = self.cfg.lock().unwrap();
            for p in parsed {
                match p {
                    super::links::ParsedLink::Route { route } => {
                        if route.kind == RouteKind::Shadowsocks
                            && taken.contains(&(route.host.to_ascii_lowercase(), route.port))
                        {
                            continue;
                        }
                        if !cfg.routes.iter().any(|r| r.id == route.id) {
                            cfg.routes.push(route.clone());
                            added.push(route);
                        }
                    }
                    super::links::ParsedLink::Unsupported { scheme, .. } => {
                        let taken_by_mihomo = mihomo_kinds.contains(&scheme)
                            || (scheme == "hy2" && mihomo_kinds.contains("hysteria2"));
                        if !(mihomo_on && taken_by_mihomo) {
                            unsupported.push(scheme)
                        }
                    }
                }
            }
        }
        self.save();
        if !nodes.is_empty() && mihomo::add_nodes(nodes) > 0 && mihomo_on {
            let me = self.clone();
            tokio::spawn(async move {
                me.mihomo_ensure(true).await;
            });
        }
        (added, unsupported)
    }

    pub fn add_route(&self, mut route: Route) -> Route {
        route.source = RouteSource::Manual;
        route.finish();
        {
            let mut cfg = self.cfg.lock().unwrap();
            cfg.routes.retain(|r| r.id != route.id);
            cfg.routes.push(route.clone());
        }
        self.save();
        route
    }

    pub fn remove_route(&self, id: &str) {
        mihomo::remove_by_route_id(id);
        {
            let mut cfg = self.cfg.lock().unwrap();
            cfg.routes.retain(|r| r.id != id);
            if cfg.fixed_route.as_deref() == Some(id) {
                cfg.fixed_route = None;
                cfg.mode = Mode::Auto;
            }
        }
        self.health.lock().unwrap().remove(id);
        self.save();
    }

    /// Removes saved routes that failed their last test.
    pub fn remove_failed(&self) -> usize {
        let health = self.health.lock().unwrap().clone();
        let mut removed_nodes = 0;
        for r in mihomo::routes() {
            if health.get(&r.id).and_then(|h| h.ok) == Some(false)
                && mihomo::remove_by_route_id(&r.id)
            {
                removed_nodes += 1;
            }
        }
        let mut cfg = self.cfg.lock().unwrap();
        let before = cfg.routes.len();
        cfg.routes
            .retain(|r| health.get(&r.id).and_then(|h| h.ok) != Some(false));
        let removed = before - cfg.routes.len();
        drop(cfg);
        self.save();
        if removed_nodes > 0 {
            // The listeners of the removed nodes go on the next reload.
            tokio::spawn(async {
                if let Err(e) = mihomo::start().await {
                    tracing::debug!("[tg-net] mihomo reload: {e:#}");
                }
            });
        }
        removed + removed_nodes
    }

    /// Saves a detected/rescue route so it survives restarts.
    pub fn keep(&self, id: &str) -> anyhow::Result<()> {
        let r = self
            .find(id)
            .ok_or_else(|| anyhow::anyhow!("unknown route"))?;
        // A mihomo listener is already kept as its node; its port is not
        // a stable thing to save.
        if mihomo::is_mihomo_route(id) {
            return Ok(());
        }
        {
            let mut cfg = self.cfg.lock().unwrap();
            if !cfg.routes.iter().any(|x| x.id == r.id) {
                cfg.routes.push(r);
            }
        }
        self.save();
        Ok(())
    }

    pub fn set_mode(&self, mode: Mode, fixed_route: Option<String>) {
        {
            let mut cfg = self.cfg.lock().unwrap();
            cfg.mode = mode;
            cfg.fixed_route = fixed_route.clone();
        }
        if let Some(id) = fixed_route {
            *self.active.lock().unwrap() = Some(id);
        }
        self.save();
    }

    pub fn set_opts(&self, opts: NetOpts, use_rescue: Option<bool>) {
        {
            let mut cfg = self.cfg.lock().unwrap();
            cfg.opts = opts;
            if let Some(v) = use_rescue {
                cfg.use_rescue = v;
            }
        }
        self.save();
    }

    pub async fn add_subscription(
        self: &Arc<Self>,
        url: &str,
        name: &str,
    ) -> anyhow::Result<SubscriptionEntry> {
        let via = self
            .detected
            .lock()
            .unwrap()
            .iter()
            .map(|d| d.route.clone())
            .find(|r| matches!(r.kind, RouteKind::Socks5 | RouteKind::Http));
        let imported = subscription::fetch(url, via.as_ref()).await?;
        let entry = SubscriptionEntry {
            url: url.to_string(),
            name: if name.is_empty() {
                host_of(url)
            } else {
                name.to_string()
            },
            updated_at: clock::system_unix(),
            count: imported.routes.len(),
            unsupported: imported.unsupported.clone(),
        };
        {
            let mut cfg = self.cfg.lock().unwrap();
            cfg.routes
                .retain(|r| !matches!(&r.source, RouteSource::Subscription { url: u } if u == url));
            cfg.routes.extend(imported.routes.clone());
            cfg.subscriptions.retain(|s| s.url != url);
            cfg.subscriptions.push(entry.clone());
        }
        self.save();
        self.probe_many(imported.routes).await;
        if !imported.unsupported.is_empty() {
            let source = RouteSource::Subscription {
                url: url.to_string(),
            };
            match mihomo::fetch_subscription_nodes(url, via.as_ref()).await {
                Ok(nodes) => {
                    mihomo::replace_source(&source, nodes);
                    self.mihomo_ensure(true).await;
                }
                Err(e) => tracing::debug!("[tg-net] mihomo nodes of {url}: {e:#}"),
            }
        }
        Ok(entry)
    }

    pub fn remove_subscription(&self, url: &str) {
        mihomo::remove_source(&RouteSource::Subscription {
            url: url.to_string(),
        });
        {
            let mut cfg = self.cfg.lock().unwrap();
            cfg.routes
                .retain(|r| !matches!(&r.source, RouteSource::Subscription { url: u } if u == url));
            cfg.subscriptions.retain(|s| s.url != url);
        }
        self.save();
    }

    pub async fn refresh_subscriptions(self: &Arc<Self>) -> Vec<(String, Result<usize, String>)> {
        let subs = self.cfg.lock().unwrap().subscriptions.clone();
        let mut out = Vec::new();
        for s in subs {
            let res = self.add_subscription(&s.url, &s.name).await;
            out.push((s.url, res.map(|e| e.count).map_err(|e| format!("{e:#}"))));
        }
        out
    }

    /// Proxies that can reach GitHub for the mihomo download: every
    /// detected or saved SOCKS5/HTTP route, working ones first.
    fn download_proxies(&self) -> Vec<String> {
        let health = self.health.lock().unwrap().clone();
        let mut routes: Vec<Route> = self
            .all_routes()
            .into_iter()
            .map(|(r, _)| r)
            .filter(|r| matches!(r.kind, RouteKind::Socks5 | RouteKind::Http))
            .filter(|r| !mihomo::is_mihomo_route(&r.id))
            .collect();
        routes.sort_by_key(|r| match health.get(&r.id).and_then(|h| h.ok) {
            Some(true) => 0,
            None => 1,
            Some(false) => 2,
        });
        routes.iter().map(mihomo::proxy_url).collect()
    }

    /// Brings the sidecar in line with the setting: stopped when off or
    /// without nodes, started (or reloaded when `reload`) when on. A plain
    /// scan (`reload == false`) never waits for the 20 MB download: that
    /// runs in the background.
    async fn mihomo_ensure(self: &Arc<Self>, reload: bool) {
        let on = self.cfg.lock().unwrap().use_mihomo == Some(true);
        if !on || mihomo::nodes().is_empty() {
            if mihomo::is_running() {
                mihomo::stop().await;
            }
            return;
        }
        if mihomo::is_running() && !reload {
            return;
        }
        if !mihomo::installed() && !reload {
            let me = self.clone();
            tokio::spawn(async move {
                me.mihomo_bring_up(true).await;
            });
            return;
        }
        self.mihomo_bring_up(reload).await;
    }

    /// Installs when missing, starts or reloads, and probes the node
    /// routes when `probe`.
    async fn mihomo_bring_up(self: &Arc<Self>, probe: bool) {
        if !mihomo::installed() {
            if let Err(e) = mihomo::install(&self.download_proxies()).await {
                tracing::warn!("[tg-net] mihomo install failed: {e:#}");
                return;
            }
        }
        match mihomo::start().await {
            Ok(routes) => {
                if probe {
                    self.probe_many(routes).await;
                }
            }
            Err(e) => tracing::warn!("[tg-net] mihomo start failed: {e:#}"),
        }
    }

    /// Turns the mihomo sidecar on (downloading it if needed, then starting
    /// it and probing every node) or off.
    pub async fn set_use_mihomo(
        self: &Arc<Self>,
        on: bool,
    ) -> anyhow::Result<mihomo::MihomoStatus> {
        self.cfg.lock().unwrap().use_mihomo = Some(on);
        self.save();
        if on {
            if !mihomo::installed() {
                mihomo::install(&self.download_proxies()).await?;
            }
            if !mihomo::nodes().is_empty() {
                let routes = mihomo::start().await?;
                self.probe_many(routes).await;
            }
        } else {
            mihomo::stop().await;
        }
        Ok(self.mihomo_status())
    }

    pub fn mihomo_status(&self) -> mihomo::MihomoStatus {
        mihomo::status(self.cfg.lock().unwrap().use_mihomo)
    }

    /// Downloads the pinned binary without turning anything on.
    pub async fn mihomo_install(&self) -> anyhow::Result<mihomo::MihomoStatus> {
        mihomo::install(&self.download_proxies()).await?;
        Ok(self.mihomo_status())
    }

    /// Restarts (reloads) the sidecar and probes every node.
    pub async fn mihomo_restart(self: &Arc<Self>) -> anyhow::Result<mihomo::MihomoStatus> {
        let routes = mihomo::start().await?;
        self.probe_many(routes).await;
        Ok(self.mihomo_status())
    }

    /// Adds mihomo nodes from pasted text without touching native routes.
    /// Returns how many were new.
    pub async fn mihomo_add_links(self: &Arc<Self>, text: &str) -> usize {
        let added = mihomo::add_nodes(mihomo::extract_nodes(text, RouteSource::Link));
        if added > 0 {
            self.mihomo_ensure(true).await;
        }
        added
    }

    /// Removes one node by its key and reloads.
    pub async fn mihomo_remove_node(self: &Arc<Self>, key: &str) {
        if let Some(r) = mihomo::routes()
            .into_iter()
            .find(|r| mihomo::route_key(&r.id).as_deref() == Some(key))
        {
            self.health.lock().unwrap().remove(&r.id);
        }
        mihomo::remove_node(key);
        self.mihomo_ensure(true).await;
    }

    /// True once some route has been proven to work.
    pub fn has_working_route(&self) -> bool {
        self.health
            .lock()
            .unwrap()
            .values()
            .any(|h| h.ok == Some(true))
    }
}

fn source_rank(s: &RouteSource) -> u64 {
    match s {
        RouteSource::Manual | RouteSource::Link => 1,
        RouteSource::System => 2,
        RouteSource::Clash { .. } | RouteSource::LocalApp { .. } | RouteSource::Env { .. } => 3,
        RouteSource::Subscription { .. } => 4,
        RouteSource::PublicList { .. } => 5,
        RouteSource::Direct => 6,
    }
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| url.to_string())
}
