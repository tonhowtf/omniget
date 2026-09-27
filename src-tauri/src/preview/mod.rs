//! Thread "Browser": a pilotable webview per thread, anchored over the
//! Browser tab of the right panel, plus the tools agents use to read and
//! drive it, dev-server discovery and the SnapShot window capture.
//!
//! Tauri 2 multiwebview (`Window::add_child`) sits behind the `unstable`
//! feature, which this app does not enable. So each preview is its own
//! undecorated `WebviewWindow` with the main window as parent (a child window
//! on macOS, an owned window on Windows, transient on Linux), placed in screen
//! coordinates over the tab rectangle the front reports, and moved with the
//! main window. Labels are `preview-<thread>`.
//!
//! Nothing runs at rest: no timers; the main-window hook only repositions
//! visible previews when the main window moves or resizes.

pub mod capture;
pub mod ports;
pub mod script;
pub mod tools;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
};

/// Front event: `PreviewState` whenever a preview's URL, title, loading or
/// visibility changes.
pub const EVENT_STATE: &str = "preview://state";
pub const LABEL_PREFIX: &str = "preview-";
const MAIN: &str = "main";
const EVAL_TIMEOUT: Duration = Duration::from_secs(12);
const LOAD_TIMEOUT: Duration = Duration::from_secs(20);

/// A rectangle in the main webview, CSS pixels, plus the page's
/// `devicePixelRatio` (falls back to the window scale factor).
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub scale: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewState {
    pub thread_id: String,
    pub label: String,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub visible: bool,
}

struct Session {
    thread_id: String,
    url: String,
    title: String,
    loading: bool,
    visible: bool,
    rect: Option<Rect>,
}

struct Pending {
    label: String,
    tx: tokio::sync::oneshot::Sender<Value>,
}

#[derive(Default)]
pub struct PreviewManager {
    sessions: Mutex<HashMap<String, Session>>,
    pending: Mutex<HashMap<String, Pending>>,
    loads: Mutex<HashMap<String, Vec<tokio::sync::oneshot::Sender<()>>>>,
    last: Mutex<Option<String>>,
    hooked: AtomicBool,
}

pub fn manager() -> &'static PreviewManager {
    static M: OnceLock<PreviewManager> = OnceLock::new();
    M.get_or_init(PreviewManager::default)
}

/// `preview-<thread id with only [A-Za-z0-9_-]>`.
pub fn label_for(thread_id: &str) -> String {
    let clean: String = thread_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(64)
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
    format!("OmniGet Preview {label}")
}

/// Only web pages; `javascript:`/`file:` and app schemes are refused.
pub fn normalize_url(raw: &str) -> Result<url::Url, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("PREVIEW_URL: empty URL".into());
    }
    let with_scheme = if raw.contains("://") || raw.starts_with("about:") {
        raw.to_string()
    } else if raw.starts_with("localhost")
        || raw.starts_with("127.")
        || raw.starts_with('[')
        || raw.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        format!("http://{raw}")
    } else if raw.starts_with(':') {
        format!("http://localhost{raw}")
    } else {
        format!("https://{raw}")
    };
    let u = url::Url::parse(&with_scheme).map_err(|e| format!("PREVIEW_URL: {e}"))?;
    match u.scheme() {
        "http" | "https" => Ok(u),
        "about" if u.as_str() == "about:blank" => Ok(u),
        s => Err(format!("PREVIEW_URL: scheme {s} is not allowed")),
    }
}

impl PreviewManager {
    fn state_of(&self, label: &str) -> Option<PreviewState> {
        let s = self.sessions.lock().ok()?;
        s.get(label).map(|x| PreviewState {
            thread_id: x.thread_id.clone(),
            label: label.to_string(),
            url: x.url.clone(),
            title: x.title.clone(),
            loading: x.loading,
            visible: x.visible,
        })
    }

    fn emit(&self, app: &AppHandle, label: &str) {
        if let Some(st) = self.state_of(label) {
            let _ = app.emit_to(MAIN, EVENT_STATE, st);
        }
    }

    fn touch(&self, label: &str) {
        if let Ok(mut l) = self.last.lock() {
            *l = Some(label.to_string());
        }
    }

    pub fn states(&self) -> Vec<PreviewState> {
        let labels: Vec<String> = self
            .sessions
            .lock()
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default();
        labels.iter().filter_map(|l| self.state_of(l)).collect()
    }

    pub fn state(&self, thread_id: &str) -> Option<PreviewState> {
        self.state_of(&label_for(thread_id))
    }

    /// The preview a tool targets: the thread's, or the last one used.
    pub fn resolve_label(&self, thread_id: Option<&str>) -> Option<String> {
        if let Some(t) = thread_id.filter(|t| !t.is_empty()) {
            let l = label_for(t);
            return self.sessions.lock().ok()?.contains_key(&l).then_some(l);
        }
        let last = self.last.lock().ok()?.clone();
        let sessions = self.sessions.lock().ok()?;
        match last {
            Some(l) if sessions.contains_key(&l) => Some(l),
            _ => sessions.keys().next().cloned(),
        }
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
        .ok_or_else(|| "PREVIEW_NO_MAIN: main window is not open".to_string())
}

fn window(app: &AppHandle, label: &str) -> Result<WebviewWindow, String> {
    app.get_webview_window(label).ok_or_else(|| "PREVIEW_NOT_OPEN: no preview for this thread; open the Browser tab or call preview_navigate with a url".to_string())
}

/// Moves the preview window over `rect` of the main webview.
fn place(app: &AppHandle, ww: &WebviewWindow, rect: &Rect) -> Result<(), String> {
    let main = main_window(app)?;
    let scale = rect
        .scale
        .filter(|s| *s > 0.0)
        .unwrap_or_else(|| main.scale_factor().unwrap_or(1.0));
    let origin = main
        .inner_position()
        .map_err(|e| format!("PREVIEW_PLACE: {e}"))?;
    let x = origin.x + (rect.x * scale).round() as i32;
    let y = origin.y + (rect.y * scale).round() as i32;
    let w = (rect.width * scale).round().max(1.0) as u32;
    let h = (rect.height * scale).round().max(1.0) as u32;
    ww.set_position(PhysicalPosition::new(x, y))
        .map_err(|e| format!("PREVIEW_PLACE: {e}"))?;
    ww.set_size(PhysicalSize::new(w, h))
        .map_err(|e| format!("PREVIEW_PLACE: {e}"))?;
    Ok(())
}

/// Repositions visible previews when the main window moves; closes them all
/// when it goes away. Installed once, on the first preview.
fn hook_main(app: &AppHandle) {
    let m = manager();
    if m.hooked.swap(true, Ordering::SeqCst) {
        return;
    }
    let Ok(main) = main_window(app) else {
        m.hooked.store(false, Ordering::SeqCst);
        return;
    };
    let app2 = app.clone();
    main.on_window_event(move |ev| match ev {
        tauri::WindowEvent::Moved(_)
        | tauri::WindowEvent::Resized(_)
        | tauri::WindowEvent::ScaleFactorChanged { .. } => {
            let visible: Vec<(String, Rect)> = manager()
                .sessions
                .lock()
                .map(|s| {
                    s.iter()
                        .filter(|(_, x)| x.visible)
                        .filter_map(|(l, x)| x.rect.map(|r| (l.clone(), r)))
                        .collect()
                })
                .unwrap_or_default();
            for (label, rect) in visible {
                if let Some(ww) = app2.get_webview_window(&label) {
                    let _ = place(&app2, &ww, &rect);
                }
            }
        }
        tauri::WindowEvent::Destroyed => {
            let labels: Vec<String> = manager()
                .sessions
                .lock()
                .map(|s| s.keys().cloned().collect())
                .unwrap_or_default();
            for l in labels {
                if let Some(ww) = app2.get_webview_window(&l) {
                    let _ = ww.destroy();
                }
            }
            if let Ok(mut s) = manager().sessions.lock() {
                s.clear();
            }
            manager().hooked.store(false, Ordering::SeqCst);
        }
        _ => {}
    });
}

fn build(
    app: &AppHandle,
    thread_id: &str,
    label: &str,
    url: url::Url,
) -> Result<WebviewWindow, String> {
    let main = main_window(app)?;
    let app_load = app.clone();
    let app_title = app.clone();
    let builder = tauri::WebviewWindowBuilder::new(app, label, WebviewUrl::External(url.clone()))
        .title(window_title(label))
        .decorations(false)
        .resizable(false)
        .skip_taskbar(true)
        .shadow(false)
        .focused(false)
        .visible(false)
        .accept_first_mouse(true)
        .inner_size(1024.0, 720.0)
        .initialization_script(script::INIT)
        .on_navigation(|u| !matches!(u.scheme(), "javascript" | "file"))
        .on_page_load(move |ww, payload| {
            let label = ww.label().to_string();
            let url = payload.url().to_string();
            let m = manager();
            match payload.event() {
                tauri::webview::PageLoadEvent::Started => m.set(&label, |s| {
                    s.loading = true;
                    s.url = url;
                }),
                tauri::webview::PageLoadEvent::Finished => {
                    m.set(&label, |s| {
                        s.loading = false;
                        s.url = url;
                    });
                    m.fire_loads(&label);
                }
            }
            m.emit(&app_load, &label);
        })
        .on_document_title_changed(move |ww, title| {
            let label = ww.label().to_string();
            manager().set(&label, |s| s.title = title);
            manager().emit(&app_title, &label);
        });
    let builder = builder
        .parent(&main)
        .map_err(|e| format!("PREVIEW_OPEN: {e}"))?;
    let ww = builder.build().map_err(|e| format!("PREVIEW_OPEN: {e}"))?;
    if let Ok(mut s) = manager().sessions.lock() {
        s.insert(
            label.to_string(),
            Session {
                thread_id: thread_id.to_string(),
                url: url.to_string(),
                title: String::new(),
                loading: true,
                visible: false,
                rect: None,
            },
        );
    }
    let label_closed = label.to_string();
    ww.on_window_event(move |ev| {
        if let tauri::WindowEvent::Destroyed = ev {
            if let Ok(mut s) = manager().sessions.lock() {
                s.remove(&label_closed);
            }
        }
    });
    hook_main(app);
    Ok(ww)
}

/// Opens (or reuses) the thread's preview. `rect` given = show it there;
/// `rect` absent = keep it hidden (agents can still drive it).
pub fn open(
    app: &AppHandle,
    thread_id: &str,
    url: Option<&str>,
    rect: Option<Rect>,
) -> Result<PreviewState, String> {
    let label = label_for(thread_id);
    let m = manager();
    let target = url.map(normalize_url).transpose()?;
    let ww = match app.get_webview_window(&label) {
        Some(ww) => {
            if let Some(u) = &target {
                let current = m.state_of(&label).map(|s| s.url).unwrap_or_default();
                if current != u.as_str() {
                    ww.navigate(u.clone())
                        .map_err(|e| format!("PREVIEW_NAVIGATE: {e}"))?;
                }
            }
            ww
        }
        None => {
            let u = target.unwrap_or_else(|| url::Url::parse("about:blank").expect("static url"));
            build(app, thread_id, &label, u)?
        }
    };
    if let Some(r) = rect {
        show_at(app, &ww, &label, r)?;
    }
    m.touch(&label);
    m.emit(app, &label);
    m.state_of(&label)
        .ok_or_else(|| "PREVIEW_OPEN: session vanished".into())
}

fn show_at(app: &AppHandle, ww: &WebviewWindow, label: &str, rect: Rect) -> Result<(), String> {
    let m = manager();
    if rect.width < 2.0 || rect.height < 2.0 {
        let _ = ww.hide();
        m.set(label, |s| {
            s.visible = false;
            s.rect = Some(rect);
        });
        return Ok(());
    }
    place(app, ww, &rect)?;
    let was_visible = ww.is_visible().unwrap_or(false);
    if !was_visible {
        ww.show().map_err(|e| format!("PREVIEW_SHOW: {e}"))?;
        // Showing a window makes it key on macOS; give the keyboard back to
        // the composer.
        if let Ok(main) = main_window(app) {
            let _ = main.set_focus();
        }
    }
    m.set(label, |s| {
        s.visible = true;
        s.rect = Some(rect);
    });
    Ok(())
}

pub fn set_bounds(app: &AppHandle, thread_id: &str, rect: Rect) -> Result<(), String> {
    let label = label_for(thread_id);
    let ww = window(app, &label)?;
    show_at(app, &ww, &label, rect)?;
    manager().emit(app, &label);
    Ok(())
}

pub fn hide(app: &AppHandle, thread_id: &str) -> Result<(), String> {
    let label = label_for(thread_id);
    if let Some(ww) = app.get_webview_window(&label) {
        let _ = ww.hide();
    }
    manager().set(&label, |s| s.visible = false);
    manager().emit(app, &label);
    Ok(())
}

/// Hides every preview (route change, panel closed).
pub fn hide_all(app: &AppHandle) {
    let labels: Vec<String> = manager()
        .sessions
        .lock()
        .map(|s| s.keys().cloned().collect())
        .unwrap_or_default();
    for l in labels {
        if let Some(ww) = app.get_webview_window(&l) {
            let _ = ww.hide();
        }
        manager().set(&l, |s| s.visible = false);
        manager().emit(app, &l);
    }
}

pub fn close(app: &AppHandle, thread_id: &str) -> Result<(), String> {
    let label = label_for(thread_id);
    if let Some(ww) = app.get_webview_window(&label) {
        ww.destroy().map_err(|e| format!("PREVIEW_CLOSE: {e}"))?;
    }
    if let Ok(mut s) = manager().sessions.lock() {
        s.remove(&label);
    }
    Ok(())
}

/// Waits for the next finished page load of `label` (after a navigate).
async fn wait_load(label: &str) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Ok(mut l) = manager().loads.lock() {
        l.entry(label.to_string()).or_default().push(tx);
    }
    tokio::time::timeout(LOAD_TIMEOUT, rx)
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
}

/// Navigates and waits for the load to finish (up to 20 s).
pub async fn navigate(app: &AppHandle, label: &str, raw: &str) -> Result<PreviewState, String> {
    let u = normalize_url(raw)?;
    let ww = window(app, label)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Ok(mut l) = manager().loads.lock() {
        l.entry(label.to_string()).or_default().push(tx);
    }
    ww.navigate(u)
        .map_err(|e| format!("PREVIEW_NAVIGATE: {e}"))?;
    let _ = tokio::time::timeout(LOAD_TIMEOUT, rx).await;
    manager().touch(label);
    manager()
        .state_of(label)
        .ok_or_else(|| "PREVIEW_NOT_OPEN: preview closed".into())
}

/// back / forward / reload through the page history.
pub async fn history(app: &AppHandle, label: &str, action: &str) -> Result<PreviewState, String> {
    let ww = window(app, label)?;
    if action == "reload" {
        let waiter = wait_load(label);
        ww.reload().map_err(|e| format!("PREVIEW_RELOAD: {e}"))?;
        let _ = waiter.await;
    } else {
        ww.eval(format!(
            "history.{}()",
            if action == "back" { "back" } else { "forward" }
        ))
        .map_err(|e| format!("PREVIEW_HISTORY: {e}"))?;
        tokio::time::sleep(Duration::from_millis(600)).await;
    }
    manager().touch(label);
    manager()
        .state_of(label)
        .ok_or_else(|| "PREVIEW_NOT_OPEN: preview closed".into())
}

/// Runs `func` (a JS function expression) with `args` in the page and returns
/// its value. Errors thrown in the page come back as `PREVIEW_JS: ...`.
pub async fn eval_fn(
    app: &AppHandle,
    label: &str,
    func: &str,
    args: Value,
) -> Result<Value, String> {
    let ww = window(app, label)?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let (tx, rx) = tokio::sync::oneshot::channel();
    manager()
        .pending
        .lock()
        .map_err(|_| "PREVIEW_STATE: lock poisoned".to_string())?
        .insert(
            id.clone(),
            Pending {
                label: label.to_string(),
                tx,
            },
        );
    if let Err(e) = ww.eval(script::wrap(&id, func, &args)) {
        manager().pending.lock().ok().map(|mut p| p.remove(&id));
        return Err(format!("PREVIEW_EVAL: {e}"));
    }
    manager().touch(label);
    match tokio::time::timeout(EVAL_TIMEOUT, rx).await {
        Ok(Ok(v)) => {
            if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
                Ok(v.get("value").cloned().unwrap_or(Value::Null))
            } else {
                Err(format!(
                    "PREVIEW_JS: {}",
                    v.get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or("script failed")
                ))
            }
        }
        _ => {
            manager().pending.lock().ok().map(|mut p| p.remove(&id));
            Err(
                "PREVIEW_TIMEOUT: the page did not answer (still loading, or it blocks the bridge)"
                    .into(),
            )
        }
    }
}

/// Entry for `preview_report` (called by the injected script). `label` is the
/// real label of the calling webview, never a value from the page.
pub fn report(app: &AppHandle, label: &str, kind: &str, id: Option<String>, data: Value) {
    if !label.starts_with(LABEL_PREFIX) {
        return;
    }
    let m = manager();
    match kind {
        "result" => {
            let Some(id) = id else { return };
            let hit = m.pending.lock().ok().and_then(|mut p| match p.get(&id) {
                Some(x) if x.label == label => p.remove(&id),
                _ => None,
            });
            if let Some(p) = hit {
                let _ = p.tx.send(data);
            }
        }
        "status" => {
            let url = data.get("url").and_then(|v| v.as_str()).map(String::from);
            let title = data.get("title").and_then(|v| v.as_str()).map(String::from);
            m.set(label, |s| {
                if let Some(u) = url {
                    s.url = u;
                }
                if let Some(t) = title {
                    s.title = t;
                }
            });
            m.emit(app, label);
        }
        _ => {}
    }
}

/// PNG of the preview. macOS: `screencapture -l` on the preview window (shown
/// for a moment when hidden). Windows/Linux: the screen rectangle under it.
pub async fn screenshot(app: &AppHandle, label: &str) -> Result<capture::Captured, String> {
    let ww = window(app, label)?;
    let was_visible = ww.is_visible().unwrap_or(false);
    if !was_visible {
        ww.show().map_err(|e| format!("PREVIEW_SHOW: {e}"))?;
        tokio::time::sleep(Duration::from_millis(350)).await;
    }
    let result = shoot(&ww, label).await;
    if !was_visible {
        let _ = ww.hide();
        if let Ok(main) = main_window(app) {
            let _ = main.set_focus();
        }
    }
    result
}

#[cfg(target_os = "macos")]
async fn shoot(_ww: &WebviewWindow, label: &str) -> Result<capture::Captured, String> {
    let title = window_title(label);
    let number = tauri::async_runtime::spawn_blocking(move || capture::own_window_number(&title))
        .await
        .map_err(|e| format!("PREVIEW_SCREENSHOT: {e}"))?
        .ok_or("PREVIEW_SCREENSHOT: preview window not found in the window list")?;
    capture::capture_own_window(number, "preview").await
}

#[cfg(not(target_os = "macos"))]
async fn shoot(ww: &WebviewWindow, _label: &str) -> Result<capture::Captured, String> {
    let pos = ww
        .inner_position()
        .map_err(|e| format!("PREVIEW_SCREENSHOT: {e}"))?;
    let size = ww
        .inner_size()
        .map_err(|e| format!("PREVIEW_SCREENSHOT: {e}"))?;
    capture::capture_rect(pos.x, pos.y, size.width, size.height, "preview").await
}

/// JSON for tools: state + a hint when nothing is open.
pub fn state_json(label: &str) -> Value {
    manager()
        .state_of(label)
        .map(|s| json!(s))
        .unwrap_or(Value::Null)
}
