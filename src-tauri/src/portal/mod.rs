//! Canvas portals: a real browser per portal node (`PortalHost`), keyed by
//! the node id, placed over the node's body when the canvas says it is
//! "live" and hidden otherwise (the canvas then draws the last native
//! snapshot). Agents corded to the portal drive it through
//! `crate::canvas::portal` (snapshot with `@eN` refs, click, fill, press...).
//!
//! Hosting (02-motor-do-canvas §7.2, phase 1): Tauri's multiwebview
//! (`Window::add_child`) is behind the `unstable` feature, which the app does
//! not enable, so each portal is an undecorated `WebviewWindow` parented to
//! the main window, like the thread preview (`crate::preview`), whose
//! injected script and automation functions it reuses. Phase 2 swaps the
//! window for a child webview behind this same interface.
//!
//! Labels are `portal-<nodeId>`; `ipc_guard` lets those pages call only
//! `canvas_portal_report`. Storage is scoped per portal (`storage`).
//!
//! Nothing runs at rest: no timers. The main-window hook repositions visible
//! portals when the main window moves; a canvas-store listener closes the
//! portal of a removed node.

pub mod automation;
pub mod script;
pub mod storage;
pub mod tls;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewUrl,
    WebviewWindow,
};

pub use storage::Scope;

/// `PortalState` whenever a portal's URL, title, loading or visibility changes.
pub const EVENT_STATE: &str = "canvas://portal-state";
/// `{nodeId, by, method, at}` each time an agent drives a portal.
pub const EVENT_DRIVE: &str = "canvas://portal-drive";
/// `{nodeId}` when the portal's webview went away (closed or unloaded).
pub const EVENT_CLOSED: &str = "canvas://portal-closed";
pub const LABEL_PREFIX: &str = "portal-";
const MAIN: &str = "main";
const EVAL_TIMEOUT: Duration = Duration::from_secs(15);
const LOAD_TIMEOUT: Duration = Duration::from_secs(20);
/// Hidden portals keep this viewport unless the node says otherwise.
const DEFAULT_VIEWPORT: (f64, f64) = (1024.0, 720.0);

/// Where the portal goes, in CSS px of the main webview, plus the page's
/// `devicePixelRatio` and the canvas zoom (page zoom of the portal, so its
/// layout width stays the node's width in CSS px).
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub scale: Option<f64>,
    #[serde(default)]
    pub zoom: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalState {
    pub node_id: String,
    pub label: String,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub visible: bool,
    pub scope: Scope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// Viewport of the page in CSS px (what the agent's snapshot sees).
    pub viewport: (f64, f64),
    /// Host whose untrusted certificate this portal refused (the node shows
    /// the warning and "Proceed", see [`trust_cert`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_error: Option<String>,
    pub muted: bool,
    /// Portal that opened this one with `window.open` (login popups).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opener_id: Option<String>,
    /// Portal whose cookies and storage this one uses (portal↔portal cord).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shares_session_with: Option<String>,
}

/// What `open` needs; the canvas command fills it from the node.
#[derive(Debug, Clone, Default)]
pub struct OpenSpec {
    pub node_id: String,
    pub workspace_id: String,
    pub url: Option<String>,
    pub scope: Scope,
    pub user_agent: Option<String>,
    /// Page viewport while hidden (the node body, CSS px).
    pub viewport: Option<(f64, f64)>,
    /// Hosts whose self-signed certificate the user accepted (node data).
    pub trusted_hosts: Vec<String>,
    pub muted: bool,
}

struct Session {
    /// Build generation: a rebuilt window (new User-Agent) must not be
    /// dropped by the old window's late `Destroyed` event.
    generation: u64,
    node_id: String,
    workspace_id: String,
    url: String,
    title: String,
    loading: bool,
    visible: bool,
    bounds: Option<Bounds>,
    zoom: f64,
    scope: Scope,
    user_agent: Option<String>,
    viewport: (f64, f64),
    /// Store key in use (see `storage::effective_key`); a change rebuilds.
    store_key: String,
    opener: Option<String>,
    muted: bool,
}

struct Pending {
    tx: tokio::sync::oneshot::Sender<Value>,
}

#[derive(Default)]
pub struct PortalHost {
    sessions: Mutex<HashMap<String, Session>>,
    pending: Mutex<HashMap<String, Pending>>,
    loads: Mutex<HashMap<String, Vec<tokio::sync::oneshot::Sender<()>>>>,
    hooked: AtomicBool,
    app: OnceLock<AppHandle>,
    store_listener: AtomicBool,
}

pub fn host() -> &'static PortalHost {
    static H: OnceLock<PortalHost> = OnceLock::new();
    H.get_or_init(PortalHost::default)
}

/// Hands the host the app handle (agents' RPC has none) and closes portals
/// whose node is removed from the canvas. Idempotent; call from setup and
/// from every portal command.
pub fn bind(app: &AppHandle) {
    let h = host();
    let _ = h.app.set(app.clone());
    // Before any portal exists: WebKit reads the delegate's methods when a
    // webview is created (self-signed HTTPS, see `tls`).
    if let Ok(main) = main_window(app) {
        tls::install(&main);
    }
    if h.store_listener.swap(true, Ordering::SeqCst) {
        return;
    }
    let app2 = app.clone();
    crate::canvas::doc::host().store().subscribe(move |ev| {
        use omniget_core::core::canvas::ops::Op;
        use omniget_core::core::canvas::store::StoreEvent;
        let StoreEvent::Patch(p) = ev else { return };
        let mut relink = false;
        for op in &p.ops {
            match op {
                Op::RemoveNode { id } if host().has(id) => {
                    let app3 = app2.clone();
                    let id = id.clone();
                    // Never destroy a window from inside the store's lock.
                    tauri::async_runtime::spawn(async move {
                        let _ = close(&app3, &id);
                    });
                }
                Op::AddConnection { .. } | Op::RemoveConnection { .. } => relink = true,
                Op::UpdateNode { id, patch } if host().has(id) => {
                    let data = patch.get("data");
                    let scope_changed = data.is_some_and(|d| d.get("storageScope").is_some());
                    if scope_changed {
                        relink = true;
                    }
                    // `node_update data.url` (MCP, remote, menu) navigates the
                    // live page; the portal's own write-back does not.
                    let url = data
                        .and_then(|d| d.get("url"))
                        .and_then(|u| u.as_str())
                        .map(String::from);
                    if let (Some(u), false) = (url, p.origin == PERSIST_ORIGIN) {
                        let current = host().state(id).map(|s| s.url).unwrap_or_default();
                        if !u.is_empty() && u != current && scope_changed {
                            // The scope rebuild recreates the webview: it
                            // loads this URL instead of the old page.
                            if let Ok(mut m) = rebuild_urls().lock() {
                                m.insert(id.clone(), u);
                            }
                        } else if !u.is_empty() && u != current {
                            let app3 = app2.clone();
                            let id = id.clone();
                            tauri::async_runtime::spawn(async move {
                                let _ = navigate(&app3, &id, &u).await;
                            });
                        }
                    }
                    if let Some(m) = data.and_then(|d| d.get("muted")).and_then(|m| m.as_bool()) {
                        let app3 = app2.clone();
                        let id = id.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = set_muted(&app3, &id, m).await;
                        });
                    }
                }
                _ => {}
            }
        }
        if relink {
            let app3 = app2.clone();
            let ws = p.workspace_id.clone();
            tauri::async_runtime::spawn(async move {
                relink_sessions(&app3, &ws).await;
            });
        }
    });
}

/// Origin of the portal's own URL write-back (not a navigation request).
const PERSIST_ORIGIN: &str = "system:portal";

/// After a portal↔portal cord or a storage scope changed: every open portal
/// of `ws` whose store key moved is rebuilt on the new store (WebKit takes
/// the data store only at creation), keeping its page and placement.
async fn relink_sessions(app: &AppHandle, ws: &str) {
    let moved: Vec<String> = host()
        .sessions
        .lock()
        .map(|s| {
            s.values()
                .filter(|x| x.workspace_id == ws && x.opener.is_none())
                .filter(|x| {
                    storage::effective_key(x.scope, &x.node_id, ws) != x.store_key
                        || node_scope(&x.node_id).is_some_and(|sc| sc != x.scope)
                })
                .map(|x| x.node_id.clone())
                .collect()
        })
        .unwrap_or_default();
    for id in moved {
        let _ = rebuild(app, &id, None).await;
    }
    // A URL that came with a scope that did not move the store: navigate.
    let left: Vec<(String, String)> = rebuild_urls()
        .lock()
        .map(|mut m| m.drain().collect())
        .unwrap_or_default();
    for (id, u) in left {
        let _ = navigate(app, &id, &u).await;
    }
}

/// URL requested together with a storage scope change (`node_update
/// {url, storageScope}`), loaded by the rebuild instead of the old page.
fn rebuild_urls() -> &'static Mutex<HashMap<String, String>> {
    static M: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

fn node_scope(node_id: &str) -> Option<Scope> {
    let store = crate::canvas::doc::host().store();
    let (ws, floor) = store.locate_node(node_id)?;
    store
        .read(&ws, &floor, |d| {
            d.node(node_id)
                .map(|n| Scope::parse(n.data_str("storageScope")))
        })
        .ok()
        .flatten()
}

/// The bound app handle (set by `bind`).
pub fn app() -> Option<AppHandle> {
    host().app.get().cloned()
}

/// `portal-<node id with only [A-Za-z0-9_-]>`.
pub fn label_for(node_id: &str) -> String {
    let clean: String = node_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    format!(
        "{LABEL_PREFIX}{}",
        if clean.is_empty() {
            "default".into()
        } else {
            clean
        }
    )
}

fn window_title(label: &str) -> String {
    format!("OmniGet Portal {label}")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Same rules as the preview: http(s) and about:blank only.
pub fn normalize_url(raw: &str) -> Result<url::Url, String> {
    crate::preview::normalize_url(raw).map_err(|e| e.replacen("PREVIEW_URL", "PORTAL_URL", 1))
}

/// Presets for `portal.ua` (Maestri's names).
pub fn user_agent_preset(name: &str) -> Option<&'static str> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "ios" | "iphone" | "safari-ios" => "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Mobile/15E148 Safari/604.1",
        "android" | "chrome-android" => "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Mobile Safari/537.36",
        "firefox-android" => "Mozilla/5.0 (Android 14; Mobile; rv:138.0) Gecko/138.0 Firefox/138.0",
        "edge-android" => "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Mobile Safari/537.36 EdgA/136.0.0.0",
        "chrome" => "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
        "firefox" => "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:138.0) Gecko/20100101 Firefox/138.0",
        "edge" => "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36 Edg/136.0.0.0",
        "safari" => "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Safari/605.1.15",
        _ => return None,
    })
}

impl PortalHost {
    pub fn has(&self, node_id: &str) -> bool {
        self.sessions
            .lock()
            .map(|s| s.contains_key(&label_for(node_id)))
            .unwrap_or(false)
    }

    fn state_of(&self, label: &str) -> Option<PortalState> {
        let s = self.sessions.lock().ok()?;
        s.get(label).map(|x| PortalState {
            node_id: x.node_id.clone(),
            label: label.to_string(),
            url: x.url.clone(),
            title: x.title.clone(),
            loading: x.loading,
            visible: x.visible,
            scope: x.scope,
            user_agent: x.user_agent.clone(),
            viewport: x.viewport,
            cert_error: tls::cert_error(label).map(|(h, _)| h),
            muted: x.muted,
            opener_id: x.opener.clone(),
            shares_session_with: x
                .store_key
                .strip_prefix("portal:node:")
                .filter(|root| *root != x.node_id)
                .map(String::from),
        })
    }

    fn emit(&self, app: &AppHandle, label: &str) {
        if let Some(st) = self.state_of(label) {
            let _ = app.emit_to(MAIN, EVENT_STATE, st);
        }
    }

    pub fn state(&self, node_id: &str) -> Option<PortalState> {
        self.state_of(&label_for(node_id))
    }

    pub fn states(&self) -> Vec<PortalState> {
        let labels: Vec<String> = self
            .sessions
            .lock()
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default();
        labels.iter().filter_map(|l| self.state_of(l)).collect()
    }

    fn set(&self, label: &str, f: impl FnOnce(&mut Session)) {
        if let Ok(mut s) = self.sessions.lock() {
            if let Some(x) = s.get_mut(label) {
                f(x);
            }
        }
    }

    fn fire_loads(&self, label: &str) {
        if let Ok(mut l) = self.loads.lock() {
            if let Some(v) = l.remove(label) {
                for tx in v {
                    let _ = tx.send(());
                }
            }
        }
    }
}

fn main_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window(MAIN)
        .ok_or_else(|| "PORTAL_NO_MAIN: main window is not open".to_string())
}

fn window(app: &AppHandle, label: &str) -> Result<WebviewWindow, String> {
    app.get_webview_window(label)
        .ok_or_else(|| "PORTAL_NOT_OPEN: this portal has no browser loaded".to_string())
}

fn place(app: &AppHandle, ww: &WebviewWindow, b: &Bounds) -> Result<(), String> {
    let main = main_window(app)?;
    let scale = b
        .scale
        .filter(|s| *s > 0.0)
        .unwrap_or_else(|| main.scale_factor().unwrap_or(1.0));
    let origin = main
        .inner_position()
        .map_err(|e| format!("PORTAL_PLACE: {e}"))?;
    let x = origin.x + (b.x * scale).round() as i32;
    let y = origin.y + (b.y * scale).round() as i32;
    let w = (b.width * scale).round().max(1.0) as u32;
    let h = (b.height * scale).round().max(1.0) as u32;
    ww.set_position(PhysicalPosition::new(x, y))
        .map_err(|e| format!("PORTAL_PLACE: {e}"))?;
    ww.set_size(PhysicalSize::new(w, h))
        .map_err(|e| format!("PORTAL_PLACE: {e}"))?;
    Ok(())
}

fn hook_main(app: &AppHandle) {
    let h = host();
    if h.hooked.swap(true, Ordering::SeqCst) {
        return;
    }
    let Ok(main) = main_window(app) else {
        h.hooked.store(false, Ordering::SeqCst);
        return;
    };
    let app2 = app.clone();
    main.on_window_event(move |ev| match ev {
        tauri::WindowEvent::Moved(_)
        | tauri::WindowEvent::Resized(_)
        | tauri::WindowEvent::ScaleFactorChanged { .. } => {
            let visible: Vec<(String, Bounds)> = host()
                .sessions
                .lock()
                .map(|s| {
                    s.iter()
                        .filter(|(_, x)| x.visible)
                        .filter_map(|(l, x)| x.bounds.map(|b| (l.clone(), b)))
                        .collect()
                })
                .unwrap_or_default();
            for (label, b) in visible {
                if let Some(ww) = app2.get_webview_window(&label) {
                    let _ = place(&app2, &ww, &b);
                }
            }
        }
        tauri::WindowEvent::Destroyed => {
            let labels: Vec<String> = host()
                .sessions
                .lock()
                .map(|s| s.keys().cloned().collect())
                .unwrap_or_default();
            for l in labels {
                if let Some(ww) = app2.get_webview_window(&l) {
                    let _ = ww.destroy();
                }
            }
            if let Ok(mut s) = host().sessions.lock() {
                s.clear();
            }
            host().hooked.store(false, Ordering::SeqCst);
        }
        _ => {}
    });
}

/// Writes the page URL back to the node (`data.url`) so a reopened canvas
/// comes back where the portal was. Skipped when unchanged.
fn persist_url(node_id: &str, url: &str) {
    if url.is_empty() || url == "about:blank" {
        return;
    }
    let store = crate::canvas::doc::host().store();
    let Some((ws, floor)) = store.locate_node(node_id) else {
        return;
    };
    let same = store
        .read(&ws, &floor, |d| {
            d.node(node_id)
                .map(|n| n.data_str("url") == Some(url))
                .unwrap_or(true)
        })
        .unwrap_or(true);
    if same {
        return;
    }
    let op = omniget_core::core::canvas::ops::Op::update_node(
        node_id,
        json!({ "data": { "url": url } }),
    );
    let _ = store.apply(&ws, &floor, vec![op], PERSIST_ORIGIN, None);
}

fn build(
    app: &AppHandle,
    label: &str,
    spec: &OpenSpec,
    url: url::Url,
) -> Result<WebviewWindow, String> {
    build_with(app, label, spec, url, None, None)
}

/// `popup`: the `window.open` request of `opener` (same WebKit configuration,
/// so the popup shares the opener's cookies and `window.opener` works).
fn build_with(
    app: &AppHandle,
    label: &str,
    spec: &OpenSpec,
    url: url::Url,
    popup: Option<tauri::webview::NewWindowFeatures>,
    opener: Option<String>,
) -> Result<WebviewWindow, String> {
    let main = main_window(app)?;
    tls::install(&main);
    let app_load = app.clone();
    let app_title = app.clone();
    let app_popup = app.clone();
    let viewport = spec.viewport.unwrap_or(DEFAULT_VIEWPORT);
    let opener_label = label.to_string();
    let mut builder =
        tauri::WebviewWindowBuilder::new(app, label, WebviewUrl::External(url.clone()))
            .title(window_title(label))
            .decorations(false)
            .resizable(false)
            .skip_taskbar(true)
            .shadow(false)
            .focused(false)
            .visible(false)
            .accept_first_mouse(true)
            .initialization_script(script::init())
            .on_navigation(|u| {
                !matches!(
                    u.scheme(),
                    "javascript" | "file" | "tauri" | "asset" | "ipc"
                )
            })
            .on_new_window(move |u, features| {
                // Login and OAuth windows become portals next to the opener
                // ("[Parent] #2"), never free-floating OS windows.
                match popup_portal(&app_popup, &opener_label, u, features) {
                    Ok(window) => tauri::webview::NewWindowResponse::Create { window },
                    Err(e) => {
                        tracing::warn!("[portal] popup from {opener_label}: {e}");
                        tauri::webview::NewWindowResponse::Deny
                    }
                }
            })
            .on_page_load(move |ww, payload| {
                let label = ww.label().to_string();
                let url = payload.url().to_string();
                let h = host();
                match payload.event() {
                    tauri::webview::PageLoadEvent::Started => {
                        tls::clear_error(&label);
                        h.set(&label, |s| {
                            s.loading = true;
                            s.url = url;
                        })
                    }
                    tauri::webview::PageLoadEvent::Finished => {
                        let mut node = None;
                        let mut muted = false;
                        h.set(&label, |s| {
                            s.loading = false;
                            s.url = url.clone();
                            node = Some(s.node_id.clone());
                            muted = s.muted;
                        });
                        h.fire_loads(&label);
                        if muted {
                            let _ = ww.eval(script::mute_js(true));
                        }
                        if let Some(n) = node {
                            persist_url(&n, &url);
                        }
                    }
                }
                h.emit(&app_load, &label);
            })
            .on_document_title_changed(move |ww, title| {
                let label = ww.label().to_string();
                host().set(&label, |s| s.title = title);
                host().emit(&app_title, &label);
            });
    builder = builder.inner_size(viewport.0.max(50.0), viewport.1.max(50.0));
    if let Some(f) = popup {
        builder = builder.window_features(f);
    }
    // A preset name (`ios`, `android`...) or a raw User-Agent string.
    if let Some(ua) = spec.user_agent.as_deref().filter(|u| !u.trim().is_empty()) {
        builder = builder.user_agent(user_agent_preset(ua).unwrap_or(ua));
    }
    let store_key = match &opener {
        // The popup lives in the opener's configuration (and store).
        Some(o) => host()
            .sessions
            .lock()
            .ok()
            .and_then(|s| s.get(&label_for(o)).map(|x| x.store_key.clone()))
            .unwrap_or_else(|| storage::scope_key(spec.scope, o, &spec.workspace_id)),
        None => {
            let key = storage::effective_key(spec.scope, &spec.node_id, &spec.workspace_id);
            builder = storage::apply(builder, spec.scope, &key);
            key
        }
    };
    tls::set_trusted(label, spec.trusted_hosts.iter().cloned());
    let builder = builder
        .parent(&main)
        .map_err(|e| format!("PORTAL_OPEN: {e}"))?;
    let ww = builder.build().map_err(|e| format!("PORTAL_OPEN: {e}"))?;
    static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut s) = host().sessions.lock() {
        s.insert(
            label.to_string(),
            Session {
                generation,
                node_id: spec.node_id.clone(),
                workspace_id: spec.workspace_id.clone(),
                url: url.to_string(),
                title: String::new(),
                loading: true,
                visible: false,
                bounds: None,
                zoom: 1.0,
                scope: spec.scope,
                user_agent: spec.user_agent.clone(),
                viewport,
                store_key,
                opener,
                muted: spec.muted,
            },
        );
    }
    let label_closed = label.to_string();
    let app_closed = app.clone();
    ww.on_window_event(move |ev| {
        if let tauri::WindowEvent::Destroyed = ev {
            let gone = host()
                .sessions
                .lock()
                .ok()
                .and_then(|mut s| match s.get(&label_closed) {
                    Some(x) if x.generation == generation => s.remove(&label_closed),
                    _ => None,
                });
            if let Some(s) = gone {
                let _ = app_closed.emit_to(MAIN, EVENT_CLOSED, json!({ "nodeId": s.node_id }));
            }
        }
    });
    hook_main(app);
    Ok(ww)
}

/// `window.open` from the portal `opener_label`: a new portal node right of
/// the opener, corded to it, named "<Opener> #n", hosting the popup webview.
/// Runs on the main thread inside WebKit's delegate call.
fn popup_portal(
    app: &AppHandle,
    opener_label: &str,
    url: url::Url,
    features: tauri::webview::NewWindowFeatures,
) -> Result<WebviewWindow, String> {
    let (opener_id, ws, scope, ua) = {
        let s = host()
            .sessions
            .lock()
            .map_err(|_| "PORTAL_STATE: lock poisoned".to_string())?;
        let x = s.get(opener_label).ok_or("PORTAL_NOT_OPEN: opener gone")?;
        (
            x.node_id.clone(),
            x.workspace_id.clone(),
            x.scope,
            x.user_agent.clone(),
        )
    };
    let store = crate::canvas::doc::host().store();
    let (sws, floor) = store
        .locate_node(&opener_id)
        .ok_or("PORTAL_NOT_FOUND: opener node")?;
    let (name, x, y, w, h, z, count) = store.read(&sws, &floor, |d| {
        let n = d.node(&opener_id);
        let name = n
            .and_then(|n| n.data_str("name").map(String::from).or(n.title.clone()))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Portal".into());
        let count = d
            .nodes
            .iter()
            .filter(|m| m.data_str("openerId") == Some(opener_id.as_str()))
            .count();
        let z = d.nodes.iter().map(|m| m.z).fold(0.0, f64::max) + 1.0;
        (
            name,
            n.map(|n| n.x + n.w + 40.0).unwrap_or(0.0),
            n.map(|n| n.y + 40.0 * count as f64).unwrap_or(0.0),
            n.map(|n| n.w).unwrap_or(800.0),
            n.map(|n| n.h).unwrap_or(600.0),
            z,
            count,
        )
    })?;
    let id = omniget_core::core::canvas::model::new_id();
    let title = format!("{name} #{}", count + 2);
    let spec = OpenSpec {
        node_id: id.clone(),
        workspace_id: ws.clone(),
        url: Some(url.to_string()),
        scope,
        user_agent: ua,
        viewport: Some((w, (h - automation::NODE_HEADER).max(100.0))),
        trusted_hosts: Vec::new(),
        muted: false,
    };
    let label = label_for(&id);
    let ww = build_with(
        app,
        &label,
        &spec,
        url.clone(),
        Some(features),
        Some(opener_id.clone()),
    )?;
    let node: omniget_core::core::canvas::model::CanvasNode = serde_json::from_value(json!({
        "id": id, "kind": "portal", "x": x, "y": y, "w": w, "h": h, "z": z, "title": title,
        "data": {
            "url": url.to_string(), "portalKind": "web", "name": title,
            "storageScope": scope.as_str(), "openerId": opener_id,
        },
    }))
    .map_err(|e| format!("PORTAL_POPUP: {e}"))?;
    let cord: omniget_core::core::canvas::model::Connection = serde_json::from_value(json!({
        "id": omniget_core::core::canvas::model::new_id(),
        "from": opener_id, "to": id, "kind": "portal", "perms": [],
    }))
    .map_err(|e| format!("PORTAL_POPUP: {e}"))?;
    // Not from inside WebKit's delegate call: the store notifies listeners.
    tauri::async_runtime::spawn(async move {
        use omniget_core::core::canvas::ops::Op;
        let _ = crate::canvas::doc::host().store().apply(
            &sws,
            &floor,
            vec![Op::AddNode { node }, Op::AddConnection { connection: cord }],
            PERSIST_ORIGIN,
            None,
        );
    });
    Ok(ww)
}

/// Opens (or reuses) the portal of `spec.node_id`, hidden. A different `url`
/// on an open portal navigates it.
pub fn open(app: &AppHandle, spec: OpenSpec) -> Result<PortalState, String> {
    bind(app);
    let label = label_for(&spec.node_id);
    let target = spec
        .url
        .as_deref()
        .filter(|u| !u.trim().is_empty())
        .map(normalize_url)
        .transpose()?;
    match app.get_webview_window(&label) {
        Some(ww) => {
            if let Some(u) = &target {
                let current = host().state_of(&label).map(|s| s.url).unwrap_or_default();
                if current != u.as_str() {
                    ww.navigate(u.clone())
                        .map_err(|e| format!("PORTAL_NAVIGATE: {e}"))?;
                }
            }
            if let Some(v) = spec.viewport {
                set_viewport(app, &spec.node_id, v.0, v.1)?;
            }
        }
        None => {
            let u = target.unwrap_or_else(|| url::Url::parse("about:blank").expect("static url"));
            build(app, &label, &spec, u)?;
        }
    }
    host().emit(app, &label);
    host()
        .state_of(&label)
        .ok_or_else(|| "PORTAL_OPEN: session vanished".into())
}

/// Shows the portal over `bounds` (live) or hides it (`None`, or a rect too
/// small to matter).
pub fn set_bounds(app: &AppHandle, node_id: &str, bounds: Option<Bounds>) -> Result<(), String> {
    let label = label_for(node_id);
    let ww = window(app, &label)?;
    let h = host();
    let Some(b) = bounds.filter(|b| b.width >= 2.0 && b.height >= 2.0) else {
        let _ = ww.hide();
        h.set(&label, |s| s.visible = false);
        h.emit(app, &label);
        return Ok(());
    };
    place(app, &ww, &b)?;
    let zoom = b.zoom.filter(|z| *z > 0.05).unwrap_or(1.0);
    let old_zoom = h
        .sessions
        .lock()
        .ok()
        .and_then(|s| s.get(&label).map(|x| x.zoom))
        .unwrap_or(1.0);
    if (zoom - old_zoom).abs() > 1e-3 {
        ww.set_zoom(zoom).map_err(|e| format!("PORTAL_ZOOM: {e}"))?;
    }
    let was_visible = ww.is_visible().unwrap_or(false);
    if !was_visible {
        ww.show().map_err(|e| format!("PORTAL_SHOW: {e}"))?;
        // Showing a window makes it key on macOS; the canvas keeps the keyboard.
        if let Ok(main) = main_window(app) {
            let _ = main.set_focus();
        }
    }
    h.set(&label, |s| {
        s.visible = true;
        s.bounds = Some(b);
        s.zoom = zoom;
        s.viewport = (b.width / zoom, b.height / zoom);
    });
    h.emit(app, &label);
    Ok(())
}

/// Page viewport (CSS px) while the portal is hidden: the node body, or the
/// size an agent asked for with `portal.resize`.
pub fn set_viewport(app: &AppHandle, node_id: &str, w: f64, h: f64) -> Result<(), String> {
    let label = label_for(node_id);
    let ww = window(app, &label)?;
    let (w, h) = (w.clamp(100.0, 4096.0), h.clamp(100.0, 4096.0));
    let visible = host()
        .sessions
        .lock()
        .ok()
        .and_then(|s| s.get(&label).map(|x| x.visible))
        .unwrap_or(false);
    if !visible {
        // Page zoom back to 1 so the viewport is exactly w×h CSS px.
        let _ = ww.set_zoom(1.0);
        ww.set_size(LogicalSize::new(w, h))
            .map_err(|e| format!("PORTAL_RESIZE: {e}"))?;
        host().set(&label, |s| {
            s.zoom = 1.0;
            s.viewport = (w, h);
        });
    }
    host().emit(app, &label);
    Ok(())
}

pub fn hide_all(app: &AppHandle) {
    let labels: Vec<String> = host()
        .sessions
        .lock()
        .map(|s| s.keys().cloned().collect())
        .unwrap_or_default();
    for l in labels {
        if let Some(ww) = app.get_webview_window(&l) {
            let _ = ww.hide();
        }
        host().set(&l, |s| s.visible = false);
        host().emit(app, &l);
    }
}

/// Destroys the webview (frees its content process). The node keeps its
/// URL; the next `open` starts it again.
pub fn close(app: &AppHandle, node_id: &str) -> Result<(), String> {
    let label = label_for(node_id);
    let opener = host()
        .sessions
        .lock()
        .ok()
        .and_then(|s| s.get(&label).map(|x| x.opener.clone()));
    match opener {
        // A popup shares its opener's WebKit configuration: destroying it
        // would unregister the IPC handler the opener uses too. It is hidden
        // and retired, and goes away with its opener.
        Some(Some(op)) => {
            if let Some(ww) = app.get_webview_window(&label) {
                let _ = ww.hide();
            }
            if let Ok(mut r) = retired().lock() {
                r.entry(op).or_default().push(label.clone());
            }
        }
        _ => {
            // Retired popups first, then the opener.
            let kids = retired()
                .lock()
                .ok()
                .and_then(|mut r| r.remove(node_id))
                .unwrap_or_default();
            for k in kids {
                if let Some(ww) = app.get_webview_window(&k) {
                    let _ = ww.destroy();
                }
            }
            if let Some(ww) = app.get_webview_window(&label) {
                ww.destroy().map_err(|e| format!("PORTAL_CLOSE: {e}"))?;
            }
        }
    }
    let gone = host()
        .sessions
        .lock()
        .ok()
        .and_then(|mut s| s.remove(&label));
    if gone.is_some() {
        let _ = app.emit_to(MAIN, EVENT_CLOSED, json!({ "nodeId": node_id }));
    }
    Ok(())
}

/// opener node id -> labels of its closed (hidden) popup webviews.
fn retired() -> &'static Mutex<HashMap<String, Vec<String>>> {
    static R: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

async fn wait_load(label: &str) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Ok(mut l) = host().loads.lock() {
        l.entry(label.to_string()).or_default().push(tx);
    }
    tokio::time::timeout(LOAD_TIMEOUT, rx)
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
}

/// Navigates and waits for the load (up to 20 s).
pub async fn navigate(app: &AppHandle, node_id: &str, raw: &str) -> Result<PortalState, String> {
    let label = label_for(node_id);
    let u = normalize_url(raw)?;
    let ww = window(app, &label)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Ok(mut l) = host().loads.lock() {
        l.entry(label.clone()).or_default().push(tx);
    }
    ww.navigate(u)
        .map_err(|e| format!("PORTAL_NAVIGATE: {e}"))?;
    let _ = tokio::time::timeout(LOAD_TIMEOUT, rx).await;
    host()
        .state_of(&label)
        .ok_or_else(|| "PORTAL_NOT_OPEN: portal closed".into())
}

/// back | forward | reload.
pub async fn history(app: &AppHandle, node_id: &str, action: &str) -> Result<PortalState, String> {
    let label = label_for(node_id);
    let ww = window(app, &label)?;
    if action == "reload" {
        let waiter = wait_load(&label);
        ww.reload().map_err(|e| format!("PORTAL_RELOAD: {e}"))?;
        let _ = waiter.await;
    } else {
        ww.eval(if action == "forward" {
            "history.forward()"
        } else {
            "history.back()"
        })
        .map_err(|e| format!("PORTAL_HISTORY: {e}"))?;
        tokio::time::sleep(Duration::from_millis(600)).await;
    }
    host()
        .state_of(&label)
        .ok_or_else(|| "PORTAL_NOT_OPEN: portal closed".into())
}

/// Runs `func` (a JS function expression) with `args` in the page.
pub async fn eval_fn(
    app: &AppHandle,
    node_id: &str,
    func: &str,
    args: Value,
) -> Result<Value, String> {
    let label = label_for(node_id);
    let ww = window(app, &label)?;
    // The certificate warning is not a page: nothing would ever answer.
    cert_blocked(&label)?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let (tx, rx) = tokio::sync::oneshot::channel();
    host()
        .pending
        .lock()
        .map_err(|_| "PORTAL_STATE: lock poisoned".to_string())?
        .insert(id.clone(), Pending { tx });
    if let Err(e) = ww.eval(script::wrap(&id, func, &args)) {
        host().pending.lock().ok().map(|mut p| p.remove(&id));
        return Err(format!("PORTAL_EVAL: {e}"));
    }
    match tokio::time::timeout(EVAL_TIMEOUT, rx).await {
        Ok(Ok(v)) => {
            if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
                Ok(v.get("value").cloned().unwrap_or(Value::Null))
            } else {
                Err(format!(
                    "PORTAL_JS: {}",
                    v.get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or("script failed")
                ))
            }
        }
        _ => {
            host().pending.lock().ok().map(|mut p| p.remove(&id));
            cert_blocked(&label)?;
            Err(
                "PORTAL_TIMEOUT: the page did not answer (still loading, or it blocks the bridge)"
                    .into(),
            )
        }
    }
}

/// `PORTAL_CERT_UNTRUSTED` while the portal shows the certificate warning.
fn cert_blocked(label: &str) -> Result<(), String> {
    match tls::cert_error(label) {
        Some((h, _)) => Err(format!(
            "PORTAL_CERT_UNTRUSTED: {h} has a certificate this portal does not trust; \
             the user must choose \"Proceed in this portal\" (or `portal act trust`)"
        )),
        None => Ok(()),
    }
}

/// Entry for `canvas_portal_report`. `label` is the calling webview's real
/// label (from Tauri), never a value from the page.
pub fn report(app: &AppHandle, label: &str, kind: &str, id: Option<String>, data: Value) {
    if !label.starts_with(LABEL_PREFIX) {
        return;
    }
    let h = host();
    match kind {
        "result" => {
            let Some(id) = id else { return };
            let hit = h.pending.lock().ok().and_then(|mut p| match p.get(&id) {
                // A popup answers through its opener's IPC handler (shared
                // WebKit configuration): the random id is the match.
                Some(_) => p.remove(&id),
                _ => None,
            });
            if let Some(p) = hit {
                let _ = p.tx.send(data);
            }
        }
        "status" => {
            let url = data.get("url").and_then(|v| v.as_str()).map(String::from);
            let title = data.get("title").and_then(|v| v.as_str()).map(String::from);
            h.set(label, |s| {
                if let Some(u) = url {
                    s.url = u;
                }
                if let Some(t) = title {
                    s.title = t;
                }
            });
            h.emit(app, label);
        }
        // `window.close()` in a popup portal (WebKit's delegate has no close
        // hook here): the popup's node goes away, which closes its webview.
        "close" => {
            // A popup shares its opener's WebKit configuration, IPC handler
            // included, so the call may carry the opener's label: find the
            // popup by opener and URL then.
            let url = data.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let popup = h.sessions.lock().ok().and_then(|s| {
                if let Some(x) = s.get(label).filter(|x| x.opener.is_some()) {
                    return Some(x.node_id.clone());
                }
                if data.get("popup").and_then(|v| v.as_bool()) != Some(true) {
                    return None;
                }
                let opener = s.get(label).map(|x| x.node_id.clone())?;
                let kids: Vec<&Session> = s
                    .values()
                    .filter(|x| x.opener.as_deref() == Some(opener.as_str()))
                    .collect();
                kids.iter()
                    .find(|x| x.url == url)
                    .or_else(|| kids.iter().max_by_key(|x| x.generation))
                    .map(|x| x.node_id.clone())
            });
            if let Some(id) = popup {
                tauri::async_runtime::spawn(async move {
                    remove_node(&id);
                });
            }
        }
        _ => {}
    }
}

fn remove_node(node_id: &str) {
    let store = crate::canvas::doc::host().store();
    if let Some((ws, floor)) = store.locate_node(node_id) {
        let op = omniget_core::core::canvas::ops::Op::RemoveNode {
            id: node_id.to_string(),
        };
        let _ = store.apply(&ws, &floor, vec![op], PERSIST_ORIGIN, None);
    }
}

/// "Proceed" on the certificate warning: this portal accepts the refused
/// host from now on (kept in the node's `data.trustedHosts`) and reloads.
pub async fn trust_cert(app: &AppHandle, node_id: &str) -> Result<PortalState, String> {
    let label = label_for(node_id);
    let (host_name, url) = tls::cert_error(&label)
        .ok_or("PORTAL_NO_CERT_ERROR: this portal refused no certificate")?;
    tls::trust(&label, &host_name);
    let store = crate::canvas::doc::host().store();
    if let Some((ws, floor)) = store.locate_node(node_id) {
        let mut hosts: Vec<String> = store
            .read(&ws, &floor, |d| {
                d.node(node_id)
                    .and_then(|n| n.data.get("trustedHosts").cloned())
                    .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        if !hosts.iter().any(|h| h.eq_ignore_ascii_case(&host_name)) {
            hosts.push(host_name.to_ascii_lowercase());
            let op = omniget_core::core::canvas::ops::Op::update_node(
                node_id,
                json!({ "data": { "trustedHosts": hosts } }),
            );
            let _ = store.apply(&ws, &floor, vec![op], PERSIST_ORIGIN, None);
        }
    }
    let target = if url.is_empty() || url == "about:blank" {
        format!("https://{host_name}/")
    } else {
        url
    };
    automation::ensure_open(app, node_id)?;
    navigate(app, node_id, &target).await
}

/// Mutes or unmutes every audio and video element of the page (reapplied on
/// each load while muted).
pub async fn set_muted(app: &AppHandle, node_id: &str, muted: bool) -> Result<PortalState, String> {
    let label = label_for(node_id);
    host().set(&label, |s| s.muted = muted);
    if let Some(ww) = app.get_webview_window(&label) {
        ww.eval(script::mute_js(muted))
            .map_err(|e| format!("PORTAL_MUTE: {e}"))?;
    }
    host().emit(app, &label);
    host()
        .state_of(&label)
        .ok_or_else(|| "PORTAL_NOT_OPEN: this portal has no browser loaded".into())
}

/// Opens the Web Inspector of the portal page (the engine's devtools).
#[cfg(debug_assertions)]
pub fn open_inspector(app: &AppHandle, node_id: &str) -> Result<(), String> {
    let ww = window(app, &label_for(node_id))?;
    ww.open_devtools();
    Ok(())
}

#[cfg(not(debug_assertions))]
pub fn open_inspector(app: &AppHandle, node_id: &str) -> Result<(), String> {
    window(app, &label_for(node_id))?;
    Err("CAPABILITY_UNAVAILABLE: the Web Inspector needs a development build".into())
}

/// Rebuilds the webview with another User-Agent (WebKit takes it only at
/// creation), keeping URL, scope and placement. `None` = engine default.
pub async fn set_user_agent(
    app: &AppHandle,
    node_id: &str,
    ua: Option<String>,
) -> Result<PortalState, String> {
    rebuild(app, node_id, Some(ua)).await
}

/// Rebuilds the webview (new User-Agent, or a store that changed with a
/// portal cord or the storage scope), keeping URL and placement. `ua: None`
/// keeps the current User-Agent; the scope and trusted hosts come from the
/// node.
pub async fn rebuild(
    app: &AppHandle,
    node_id: &str,
    ua: Option<Option<String>>,
) -> Result<PortalState, String> {
    let label = label_for(node_id);
    let fresh = automation::spec_from_node(node_id);
    let requested = rebuild_urls()
        .lock()
        .ok()
        .and_then(|mut m| m.remove(node_id));
    let (spec, url, bounds, visible) = {
        let s = host()
            .sessions
            .lock()
            .map_err(|_| "PORTAL_STATE: lock poisoned".to_string())?;
        let x = s
            .get(&label)
            .ok_or("PORTAL_NOT_OPEN: this portal has no browser loaded")?;
        (
            OpenSpec {
                node_id: x.node_id.clone(),
                workspace_id: x.workspace_id.clone(),
                url: None,
                scope: fresh.as_ref().map(|f| f.scope).unwrap_or(x.scope),
                user_agent: ua.clone().unwrap_or_else(|| x.user_agent.clone()),
                viewport: Some(x.viewport),
                trusted_hosts: fresh
                    .as_ref()
                    .map(|f| f.trusted_hosts.clone())
                    .unwrap_or_default(),
                muted: x.muted,
            },
            requested.unwrap_or_else(|| x.url.clone()),
            x.bounds,
            x.visible,
        )
    };
    if let Some(ww) = app.get_webview_window(&label) {
        let _ = ww.destroy();
    }
    if let Ok(mut s) = host().sessions.lock() {
        s.remove(&label);
    }
    // Let the old window finish going away before reusing its label.
    tokio::time::sleep(Duration::from_millis(120)).await;
    let u =
        normalize_url(&url).unwrap_or_else(|_| url::Url::parse("about:blank").expect("static url"));
    let waiter = wait_load(&label);
    build(app, &label, &spec, u)?;
    if visible {
        set_bounds(app, node_id, bounds)?;
    }
    let _ = waiter.await;
    host().emit(app, &label);
    host()
        .state_of(&label)
        .ok_or_else(|| "PORTAL_OPEN: session vanished".into())
}

/// Native pixels of the page (PNG). macOS: `WKWebView.takeSnapshot`, works
/// while hidden. Elsewhere: the screen rectangle under the window, visible
/// portals only (CapturePreview / webkit snapshot are phase 2).
pub async fn snapshot_png(app: &AppHandle, node_id: &str) -> Result<Vec<u8>, String> {
    let label = label_for(node_id);
    let ww = window(app, &label)?;
    native_png(&ww).await
}

#[cfg(target_os = "macos")]
async fn native_png(ww: &WebviewWindow) -> Result<Vec<u8>, String> {
    crate::native_capture::snapshot(ww).await
}

#[cfg(not(target_os = "macos"))]
async fn native_png(ww: &WebviewWindow) -> Result<Vec<u8>, String> {
    if !ww.is_visible().unwrap_or(false) {
        return Err(
            "CAPABILITY_UNAVAILABLE: snapshots of hidden portals need macOS for now".into(),
        );
    }
    let pos = ww
        .inner_position()
        .map_err(|e| format!("PORTAL_SNAPSHOT: {e}"))?;
    let size = ww
        .inner_size()
        .map_err(|e| format!("PORTAL_SNAPSHOT: {e}"))?;
    let shot =
        crate::preview::capture::capture_rect(pos.x, pos.y, size.width, size.height, "portal")
            .await?;
    let bytes = std::fs::read(&shot.path).map_err(|e| format!("PORTAL_SNAPSHOT: {e}"))?;
    let _ = std::fs::remove_file(&shot.path);
    Ok(bytes)
}

/// PNG written under `<app_data>/snapshots/portal-*.png`.
pub async fn screenshot(
    app: &AppHandle,
    node_id: &str,
) -> Result<crate::preview::capture::Captured, String> {
    let bytes = snapshot_png(app, node_id).await?;
    let path = crate::preview::capture::new_png_path("portal")?;
    std::fs::write(&path, &bytes).map_err(|e| format!("PORTAL_SCREENSHOT: {e}"))?;
    let (w, h) = crate::preview::capture::png_size(&bytes).unzip();
    Ok(crate::preview::capture::Captured {
        path: path.to_string_lossy().into_owned(),
        width: w,
        height: h,
        bytes: bytes.len() as u64,
    })
}

/// Tells the canvas an agent is driving `node_id` (the "agent is driving"
/// badge, and a fresh snapshot for the dormant texture).
pub fn driving(app: &AppHandle, node_id: &str, by: &str, method: &str) {
    let _ = app.emit_to(
        MAIN,
        EVENT_DRIVE,
        json!({ "nodeId": node_id, "by": by, "method": method, "at": now_ms() }),
    );
}

/// JSON of a portal's state (`null` when closed).
pub fn state_json(node_id: &str) -> Value {
    host()
        .state(node_id)
        .map(|s| json!(s))
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_safe() {
        assert_eq!(label_for("a1-b_2"), "portal-a1-b_2");
        assert_eq!(label_for("x/y z"), "portal-x_y_z");
        assert_eq!(label_for(""), "portal-default");
    }

    #[test]
    fn ua_presets() {
        assert!(user_agent_preset("iOS").unwrap().contains("iPhone"));
        assert!(user_agent_preset("android").unwrap().contains("Android"));
        assert!(user_agent_preset("nope").is_none());
    }
}
