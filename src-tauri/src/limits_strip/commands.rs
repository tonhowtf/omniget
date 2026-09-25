//! The strip window and the `limits_strip_*` commands.
//!
//! The window is created in code, in the mould of `commands::pet`: label
//! `limits-strip`, undecorated, transparent, always on top, off the taskbar,
//! with its permissions in `capabilities/limits-strip.json`. It does not exist
//! until the user switches the strip on, and the engine runs only while it
//! does.
//!
//! | code                  | meaning                                  |
//! |-----------------------|------------------------------------------|
//! | `ERR_LIMITS_WINDOW`   | a window operation failed                |
//! | `ERR_LIMITS_STORE`    | `limits-strip.json` could not be written |

use super::placement::{self, Area};
use super::prefs::{self, StripPrefs};
use super::{engine, now_ms, providers, EVENT_PREFS};
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const WINDOW_LABEL: &str = "limits-strip";
const ERR_WINDOW: &str = "ERR_LIMITS_WINDOW";

/// Moves we made ourselves are not drops: they are ignored until this time.
static OWN_MOVE_UNTIL: AtomicI64 = AtomicI64::new(0);
static EXPANDED: AtomicBool = AtomicBool::new(false);
/// Bumped by every `Moved`; a drop is a move nothing followed.
static MOVE_SEQ: AtomicU64 = AtomicU64::new(0);
const OWN_MOVE_MS: i64 = 700;
const DROP_QUIET: Duration = Duration::from_millis(450);

/// The prefs with every known provider listed, newcomers at the end.
pub fn load_adopted() -> StripPrefs {
    let mut p = prefs::load();
    p.adopt(&engine::known());
    p
}

fn rings_of(p: &StripPrefs) -> usize {
    p.providers.iter().filter(|x| p.is_on(&x.id)).count()
}

/// Work area of the monitor the strip is on, in logical pixels.
fn area_of(window: &WebviewWindow) -> Result<Area, String> {
    let monitor = window
        .current_monitor()
        .map_err(|e| format!("{ERR_WINDOW}: {e}"))?
        .or(window
            .primary_monitor()
            .map_err(|e| format!("{ERR_WINDOW}: {e}"))?)
        .ok_or_else(|| format!("{ERR_WINDOW}: no monitor"))?;
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    Ok(Area {
        x: area.position.x as f64 / scale,
        y: area.position.y as f64 / scale,
        w: area.size.width as f64 / scale,
        h: area.size.height as f64 / scale,
    })
}

/// Sizes and parks the window. Returns how far along the edge the strip sits
/// inside it, which is only non-zero while the card is open.
fn place(window: &WebviewWindow, p: &StripPrefs, expanded: bool) -> Result<f64, String> {
    let area = area_of(window)?;
    let rings = rings_of(p);
    let along = p.along_of(p.edge);
    let small = placement::window_rect(
        p.edge,
        along,
        area,
        placement::window_size(p.edge, rings, false),
    );
    let rect = if expanded {
        placement::window_rect(
            p.edge,
            along,
            area,
            placement::window_size(p.edge, rings, true),
        )
    } else {
        small
    };
    OWN_MOVE_UNTIL.store(now_ms() + OWN_MOVE_MS, Ordering::Relaxed);
    EXPANDED.store(expanded, Ordering::Relaxed);
    window
        .set_size(tauri::LogicalSize::new(rect.w, rect.h))
        .and_then(|_| window.set_position(tauri::LogicalPosition::new(rect.x, rect.y)))
        .map_err(|e| format!("{ERR_WINDOW}: {e}"))?;
    Ok(placement::strip_offset(p.edge, small, rect))
}

/// The strip was dragged and let go: glue it to the nearest edge and remember
/// the spot.
fn dropped(app: &AppHandle) {
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };
    let (Ok(pos), Ok(size), Ok(scale), Ok(area)) = (
        window.outer_position(),
        window.outer_size(),
        window.scale_factor(),
        area_of(&window),
    ) else {
        return;
    };
    let cx = (pos.x as f64 + size.width as f64 / 2.0) / scale;
    let cy = (pos.y as f64 + size.height as f64 / 2.0) / scale;
    let (edge, along) = placement::snap(cx, cy, area);
    let mut p = load_adopted();
    p.edge = edge;
    p.along.insert(edge, along);
    if let Err(error) = prefs::save(&p) {
        tracing::warn!("limits strip: {error}");
    }
    let _ = place(&window, &p, false);
    let _ = app.emit(EVENT_PREFS, &p);
    engine::set_prefs(app, p);
}

fn watch_moves(app: &AppHandle, window: &WebviewWindow) {
    let app = app.clone();
    window.on_window_event(move |event| {
        if !matches!(event, tauri::WindowEvent::Moved(_)) {
            return;
        }
        if EXPANDED.load(Ordering::Relaxed) || now_ms() < OWN_MOVE_UNTIL.load(Ordering::Relaxed) {
            return;
        }
        let seq = MOVE_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(DROP_QUIET).await;
            if MOVE_SEQ.load(Ordering::Relaxed) == seq {
                dropped(&app);
            }
        });
    });
}

/// Whether the compositor can be trusted with an alpha channel. Same reading
/// as the pet: macOS, Windows and Wayland always composite; on X11 nobody can
/// tell from here, so the page paints a solid backdrop instead of guessing.
fn transparency_trusted() -> bool {
    #[cfg(any(target_os = "macos", windows))]
    {
        true
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        std::env::var_os("WAYLAND_DISPLAY").is_some()
    }
}

fn build(app: &AppHandle, transparent: bool) -> tauri::Result<WebviewWindow> {
    let url = if transparent && transparency_trusted() {
        "/limits-strip"
    } else {
        "/limits-strip?solid=1"
    };
    let (w, h) = placement::window_size(prefs::Edge::Top, 1, false);
    WebviewWindowBuilder::new(app, WINDOW_LABEL, WebviewUrl::App(url.into()))
        .title("OmniGet")
        .inner_size(w, h)
        .resizable(false)
        .decorations(false)
        .transparent(transparent)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        // Without this the first click on macOS only focuses the window.
        .accept_first_mouse(true)
        .focused(false)
        .build()
}

fn open_window(app: &AppHandle, p: &StripPrefs) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.set_always_on_top(true);
        place(&window, p, false)?;
        return Ok(());
    }
    // A compositor with no ARGB visual refuses the transparent window; an
    // opaque strip is still a strip.
    let window = build(app, true)
        .or_else(|error| {
            tracing::warn!("limits strip: no transparent window ({error}), going opaque");
            build(app, false)
        })
        .map_err(|e| format!("{ERR_WINDOW}: {e}"))?;
    place(&window, p, false)?;
    watch_moves(app, &window);
    Ok(())
}

async fn describe(app: &AppHandle, p: &StripPrefs) -> serde_json::Value {
    let mut list = vec![json!({
        "id": providers::OMNIGET_ID,
        "label": "OmniGet",
        "local": true,
        "beta": false,
        "detected": true,
    })];
    for r in providers::all() {
        list.push(json!({
            "id": r.id(),
            "label": r.label(),
            "local": r.local(),
            "beta": r.beta(),
            // Offline and shallow: does the tool's folder exist on this OS?
            "detected": r.detect().await,
        }));
    }
    json!({
        "prefs": p,
        "open": app.get_webview_window(WINDOW_LABEL).is_some(),
        "providers": list,
    })
}

#[tauri::command]
pub async fn limits_strip_get_prefs(app: AppHandle) -> Result<serde_json::Value, String> {
    Ok(describe(&app, &load_adopted()).await)
}

/// Saves the prefs and makes the world match them: the window exists exactly
/// when the master switch is on.
#[tauri::command]
pub async fn limits_strip_set_prefs(
    app: AppHandle,
    prefs: StripPrefs,
) -> Result<serde_json::Value, String> {
    let mut p = prefs;
    p.adopt(&engine::known());
    p.thresholds = p.sane_thresholds();
    let previous = load_adopted();
    prefs::save(&p)?;
    if let Err(error) = apply(&app, &p) {
        // Restore both durable preference and engine/window state. Otherwise a
        // failed window creation leaves an enabled preference behind the UI.
        let saved = prefs::save(&previous);
        let restored = apply(&app, &previous);
        return Err(format!(
            "{error}; preference rollback: {saved:?}; runtime rollback: {restored:?}"
        ));
    }
    Ok(describe(&app, &p).await)
}

fn apply(app: &AppHandle, p: &StripPrefs) -> Result<(), String> {
    let _ = app.emit(EVENT_PREFS, p);
    if p.enabled {
        open_window(app, p)?;
        engine::set_prefs(app, p.clone());
        engine::start(app);
        engine::wake();
    } else {
        engine::stop();
        // After the stop, so the engine's copy is off too and lists no ring.
        engine::set_prefs(app, p.clone());
        if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
            window.close().map_err(|e| format!("{ERR_WINDOW}: {e}"))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn limits_strip_close(app: AppHandle) -> Result<serde_json::Value, String> {
    let mut p = load_adopted();
    p.enabled = false;
    let previous = load_adopted();
    prefs::save(&p)?;
    if let Err(error) = apply(&app, &p) {
        // Restore both durable preference and engine/window state. Otherwise a
        // failed window creation leaves an enabled preference behind the UI.
        let saved = prefs::save(&previous);
        let restored = apply(&app, &previous);
        return Err(format!(
            "{error}; preference rollback: {saved:?}; runtime rollback: {restored:?}"
        ));
    }
    Ok(describe(&app, &p).await)
}

#[tauri::command]
pub async fn limits_strip_state(app: AppHandle) -> Result<engine::Snapshot, String> {
    Ok(engine::snapshot(&app))
}

/// Asks for a fresh read of one provider, or of all. Returns the ids that
/// will be read; one read less than a minute ago is left alone.
#[tauri::command]
pub async fn limits_strip_refresh(provider_id: Option<String>) -> Result<Vec<String>, String> {
    let ids = engine::refresh(provider_id.as_deref());
    engine::wake();
    Ok(ids)
}

/// Grows the window to make room for the card, or shrinks it back.
#[tauri::command]
pub async fn limits_strip_set_expanded(
    app: AppHandle,
    expanded: bool,
) -> Result<serde_json::Value, String> {
    let window = app
        .get_webview_window(WINDOW_LABEL)
        .ok_or_else(|| format!("{ERR_WINDOW}: the strip is closed"))?;
    let p = load_adopted();
    let offset = place(&window, &p, expanded)?;
    Ok(json!({ "expanded": expanded, "offset": offset, "edge": p.edge.as_str() }))
}

/// Reopens the strip at launch when the user left it on.
pub fn restore(app: &AppHandle) {
    let p = load_adopted();
    if !p.enabled {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = apply(&app, &p) {
            tracing::warn!("limits strip did not reopen: {error}");
        }
    });
}
