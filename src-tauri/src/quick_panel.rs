use std::hash::{Hash, Hasher};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use omniget_core::core::quicktools::awake::{Awake, AwakeView, Mode};
use omniget_core::core::quicktools::history::{EntryView, History};
use omniget_core::core::quicktools::snippets::{self, Snippet, Store};
use omniget_core::core::quicktools::{clipseq, monitor, QuickError};
use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Rect, WebviewUrl,
    WebviewWindowBuilder,
};
use tauri_plugin_clipboard_manager::ClipboardExt;

pub const PANEL_LABEL: &str = "quick-panel";
pub const EVENT_VISIBLE: &str = "quick-panel://visible";
pub const EVENT_AWAKE: &str = "quick-panel://awake";
pub const EVENT_CLIPBOARD: &str = "quick-panel://clipboard";
pub const EVENT_GOTO: &str = "quick-panel://goto";

const PANEL_W: f64 = 360.0;
const PANEL_H: f64 = 640.0;
const REOPEN_GUARD_MS: u64 = 300;
const WATCH_EVERY: Duration = Duration::from_secs(1);

#[derive(Default)]
struct State {
    awake: Awake,
    awake_run: u64,
    history: History,
    watch_run: u64,
    store: Option<Store>,
    hidden_at: u64,
}

fn state() -> MutexGuard<'static, State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

static APP: OnceLock<AppHandle> = OnceLock::new();

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

pub fn setup(app: &AppHandle) {
    APP.set(app.clone()).ok();
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if store().clipboard_history {
            start_watcher(&app);
        }
    });
}

pub fn store() -> Store {
    if let Some(store) = state().store.clone() {
        return store;
    }
    let loaded = snippets::load();
    state().store.get_or_insert(loaded).clone()
}

fn commit(store: Store) -> Result<(), QuickError> {
    snippets::save(&store)?;
    state().store = Some(store);
    Ok(())
}

pub fn snippet_save(id: Option<u32>, name: &str, text: &str) -> Result<Snippet, QuickError> {
    let mut store = store();
    let saved = store.save_snippet(id, name, text)?;
    commit(store)?;
    Ok(saved)
}

pub fn snippet_delete(id: u32) -> Result<(), QuickError> {
    let mut store = store();
    store.delete_snippet(id)?;
    commit(store)
}

pub fn awake_view() -> AwakeView {
    state().awake.view(now_ms())
}

pub fn awake_set(mode: Mode) -> Result<AwakeView, QuickError> {
    let (run, wait) = {
        let mut s = state();
        let now = now_ms();
        s.awake.set(mode, now);
        s.awake_run += 1;
        (
            s.awake_run,
            s.awake.until_ms().map(|until| until.saturating_sub(now)),
        )
    };
    if !crate::core::awake::set_manual(mode != Mode::Off) {
        state().awake.set(Mode::Off, now_ms());
        announce_awake();
        return Err(QuickError::Os(
            "the system refused to stay awake".to_string(),
        ));
    }
    if let Some(wait) = wait {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(wait)).await;
            let expired = {
                let mut s = state();
                s.awake_run == run && s.awake.expire(now_ms())
            };
            if expired {
                crate::core::awake::set_manual(false);
                announce_awake();
            }
        });
    }
    announce_awake();
    Ok(awake_view())
}

fn announce_awake() {
    let Some(app) = APP.get() else {
        return;
    };
    let view = awake_view();
    crate::tray::set_awake(app, view.active.then_some(view.until_ms));
    let _ = app.emit_to(PANEL_LABEL, EVENT_AWAKE, view);
}

#[derive(Serialize)]
pub struct ClipboardView {
    pub enabled: bool,
    pub entries: Vec<EntryView>,
}

pub fn clipboard_view() -> ClipboardView {
    let enabled = store().clipboard_history;
    ClipboardView {
        enabled,
        entries: state().history.views(),
    }
}

pub fn clipboard_enable(app: &AppHandle, on: bool) -> Result<(), QuickError> {
    let mut store = store();
    if store.clipboard_history != on {
        store.clipboard_history = on;
        commit(store)?;
    }
    if on {
        start_watcher(app);
    } else {
        let mut s = state();
        s.watch_run += 1;
        s.history.wipe();
    }
    Ok(())
}

pub fn clipboard_act(app: &AppHandle, op: &str, id: Option<u64>) -> Result<(), QuickError> {
    let missing = || QuickError::NotFound("that entry is no longer in the history".to_string());
    if op == "clear" {
        state().history.clear_unpinned();
        return Ok(());
    }
    let id = id.ok_or_else(|| QuickError::InvalidInput("id is required".to_string()))?;
    match op {
        "copy" => {
            let text = state().history.text(id).map(str::to_string);
            copy_text(app, text.ok_or_else(missing)?)
        }
        "pin" | "unpin" => state()
            .history
            .set_pinned(id, op == "pin")
            .then_some(())
            .ok_or_else(missing),
        "delete" => state().history.remove(id).then_some(()).ok_or_else(missing),
        _ => Err(QuickError::InvalidInput(format!("unknown op \"{op}\""))),
    }
}

pub fn copy_text(app: &AppHandle, text: String) -> Result<(), QuickError> {
    app.clipboard()
        .write_text(text)
        .map_err(|e| QuickError::Os(e.to_string()))
}

fn fingerprint(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

async fn read_clipboard(app: &AppHandle) -> Option<String> {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || app.clipboard().read_text().ok())
        .await
        .ok()
        .flatten()
}

fn start_watcher(app: &AppHandle) {
    let run = {
        let mut s = state();
        s.watch_run += 1;
        s.watch_run
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut seen_count = clipseq::change_count();
        let mut seen_text = match seen_count {
            Some(_) => None,
            None => read_clipboard(&app).await.as_deref().map(fingerprint),
        };
        let mut tick = tokio::time::interval(WATCH_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if state().watch_run != run {
                break;
            }
            let count = clipseq::change_count();
            if count.is_some() && count == seen_count {
                continue;
            }
            seen_count = count;
            let Some(text) = read_clipboard(&app).await else {
                continue;
            };
            if count.is_none() {
                let mark = Some(fingerprint(&text));
                if mark == seen_text {
                    continue;
                }
                seen_text = mark;
            }
            let changed = {
                let mut s = state();
                s.watch_run == run && s.history.push(&text)
            };
            if changed {
                let _ = app.emit_to(PANEL_LABEL, EVENT_CLIPBOARD, ());
            }
        }
    });
}

pub fn visible(app: &AppHandle) -> bool {
    app.get_webview_window(PANEL_LABEL)
        .is_some_and(|w| w.is_visible().unwrap_or(false))
}

pub fn toggle(app: &AppHandle, anchor: Option<Rect>) {
    if visible(app) {
        hide(app);
        return;
    }
    if now_ms().saturating_sub(state().hidden_at) < REOPEN_GUARD_MS {
        return;
    }
    show(app, anchor);
}

fn anchor_or_cursor(app: &AppHandle, anchor: Option<Rect>) -> Option<Rect> {
    anchor
        .or_else(|| {
            app.tray_by_id("main-tray")
                .and_then(|t| t.rect().ok().flatten())
        })
        .or_else(|| {
            let at = app.cursor_position().ok()?;
            Some(Rect {
                position: PhysicalPosition::new(at.x, at.y).into(),
                size: PhysicalSize::new(0.0, 0.0).into(),
            })
        })
}

pub fn show(app: &AppHandle, anchor: Option<Rect>) {
    let window = match app.get_webview_window(PANEL_LABEL) {
        Some(w) => w,
        None => match crate::core::portable::with_dev_webview_dir(WebviewWindowBuilder::new(
            app,
            PANEL_LABEL,
            WebviewUrl::App("/quick-panel".into()),
        ))
        .title("OmniGet")
        .inner_size(PANEL_W, PANEL_H)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .accept_first_mouse(true)
        .visible(false)
        .build()
        {
            Ok(w) => {
                let handle = app.clone();
                w.on_window_event(move |e| {
                    if let tauri::WindowEvent::Focused(false) = e {
                        hide(&handle);
                    }
                });
                w
            }
            Err(e) => {
                tracing::warn!("quick panel: {e}");
                return;
            }
        },
    };
    if let Some(rect) = anchor_or_cursor(app, anchor) {
        crate::usage_tray::place_near(&window, rect, PANEL_W, PANEL_H);
    }
    let _ = window.show();
    let _ = window.set_focus();
    let _ = app.emit_to(PANEL_LABEL, EVENT_VISIBLE, true);
}

pub fn hide(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(PANEL_LABEL) {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
            state().hidden_at = now_ms();
            let _ = app.emit_to(PANEL_LABEL, EVENT_VISIBLE, false);
            monitor::release();
        }
    }
}

pub fn open(app: &AppHandle, target: &str) -> Result<(), QuickError> {
    let route = match target {
        "app" | "download" => None,
        "capture" => Some("/capture"),
        "dictation" => Some("/dictation"),
        "palette" => Some("/palette"),
        "ports" => Some("/devtools?tab=ports"),
        "cleaner" => Some("/washmybelly"),
        other => {
            return Err(QuickError::InvalidInput(format!(
                "unknown shortcut \"{other}\""
            )))
        }
    };
    hide(app);
    crate::tray::show_window(app);
    if let Some(route) = route {
        let _ = app.emit_to("main", EVENT_GOTO, route);
    }
    if target == "download" {
        crate::hotkey::handle_download_clipboard(app);
    }
    Ok(())
}

#[cfg(test)]
pub fn forget_store() {
    state().store = None;
}
