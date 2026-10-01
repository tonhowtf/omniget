//! The Omni pet window (Phase 5). Owned by f5-pet-window.
//!
//! A second webview window, label `pet`: 200x200, transparent, undecorated,
//! always on top, parked in one screen corner and optionally click-through.
//! The window is created in code, so
//! `tauri.conf.json` stays untouched; its permissions live in
//! `capabilities/pet.json`.
//!
//! Everything the frontend needs to know travels as JSON with stable error
//! codes, in the mould of `StreamError::code()`:
//!
//! | code                  | meaning                                           |
//! |-----------------------|---------------------------------------------------|
//! | `ERR_PET_UNSUPPORTED` | the window could not be built (no ARGB visual, no  |
//! |                       | compositor): the UI falls back to the in-app pet   |
//! | `ERR_PET_WINDOW`      | a window operation failed                          |
//! | `ERR_PET_CORNER`      | unknown corner name                                |
//! | `ERR_PET_NOT_OPEN`    | an operation needs the window and it is closed     |
//! | `ERR_PET_INTENT`      | the intent payload is not the Phase 5 contract     |
//! | `ERR_PET_STORE`       | `pet.json` could not be read or written            |
//!
//! The geometry, the preference file and the intent normalisation are pure
//! functions with tests below; the Tauri calls around them are a thin shell.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const WINDOW_LABEL: &str = "pet";
/// Event the pet window listens on. Emitted by the omni bus loop in
/// `commands::llm` after `normalize_intent`; the payload is the `Intent` of
/// `docs/agents/f5-omni-intent.md` §4.
pub const EVENT_INTENT: &str = "omni://intent";

pub const ERR_UNSUPPORTED: &str = "ERR_PET_UNSUPPORTED";
pub const ERR_WINDOW: &str = "ERR_PET_WINDOW";
pub const ERR_CORNER: &str = "ERR_PET_CORNER";
pub const ERR_NOT_OPEN: &str = "ERR_PET_NOT_OPEN";
pub const ERR_INTENT: &str = "ERR_PET_INTENT";
pub const ERR_STORE: &str = "ERR_PET_STORE";

/// Logical size of the window. The sprite is 48x64 at pivot (24, 60), so 200
/// logical pixels leave room for a 2x pet plus the speech bubble above it.
pub const PET_W: f64 = 200.0;
pub const PET_H: f64 = 200.0;
/// Gap between the window and the edges of the monitor work area, in logical
/// pixels. Big enough that the macOS Dock does not sit on top of the pet.
pub const PET_MARGIN: f64 = 16.0;

/// A line the pet says is clipped here, same bound as the Rust `Intent`.
pub const LINE_MAX_CHARS: usize = 80;

// ---------------------------------------------------------------------------
// Corner
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

impl Corner {
    pub fn as_str(self) -> &'static str {
        match self {
            Corner::TopLeft => "top-left",
            Corner::TopRight => "top-right",
            Corner::BottomLeft => "bottom-left",
            Corner::BottomRight => "bottom-right",
        }
    }
}

/// Accepts the kebab-case contract plus the snake_case and camelCase spellings
/// a frontend may produce, so a typo is the only way to get `ERR_PET_CORNER`.
pub fn parse_corner(raw: &str) -> Result<Corner, String> {
    let norm: String = raw
        .trim()
        .chars()
        .filter(|c| !matches!(c, '-' | '_' | ' '))
        .flat_map(|c| c.to_lowercase())
        .collect();
    match norm.as_str() {
        "topleft" => Ok(Corner::TopLeft),
        "topright" => Ok(Corner::TopRight),
        "bottomleft" => Ok(Corner::BottomLeft),
        "bottomright" => Ok(Corner::BottomRight),
        _ => Err(format!("{ERR_CORNER}: unknown corner {raw:?}")),
    }
}

/// A monitor work area in logical pixels: the usable rectangle, menu bar and
/// taskbar already subtracted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkArea {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Top-left logical position of a `win_w` x `win_h` window parked in `corner`
/// of `area` with `margin` of air around it.
///
/// Pure on purpose: the corner maths is the part that goes wrong on a second
/// monitor with a negative origin, and it is the part a test can reach.
/// A window larger than the work area is clamped to the area's origin instead
/// of being pushed off-screen.
pub fn corner_position(
    corner: Corner,
    area: WorkArea,
    win_w: f64,
    win_h: f64,
    margin: f64,
) -> (f64, f64) {
    let max_x = area.x + (area.w - win_w - margin).max(0.0);
    let max_y = area.y + (area.h - win_h - margin).max(0.0);
    let min_x = area.x + margin.min((area.w - win_w).max(0.0));
    let min_y = area.y + margin.min((area.h - win_h).max(0.0));
    match corner {
        Corner::TopLeft => (min_x, min_y),
        Corner::TopRight => (max_x, min_y),
        Corner::BottomLeft => (min_x, max_y),
        Corner::BottomRight => (max_x, max_y),
    }
}

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

/// What survives a restart. Deliberately tiny and deliberately its own file:
/// the pet must be able to remember its corner without the settings store,
/// which lives in a crate this module does not own.
/// Defaults, by derive: the bottom-right corner, clicks caught, pet closed —
/// so a first launch never puts a window on the user's desktop uninvited.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PetPrefs {
    pub corner: Corner,
    pub click_through: bool,
    /// Reopen the pet on the next launch.
    pub enabled: bool,
}

/// Never fails: a corrupt or half-written `pet.json` costs the user a corner,
/// not a launch, so a parse error falls back to the defaults.
pub fn prefs_from_str(raw: &str) -> PetPrefs {
    serde_json::from_str(raw).unwrap_or_default()
}

pub const PREFS_FILE: &str = "pet.json";
/// Moves the preference file, for tests and for a portable install.
pub const PREFS_DIR_ENV: &str = "OMNIGET_PET_DIR";

pub fn prefs_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os(PREFS_DIR_ENV) {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    let base = crate::core::paths::app_data_dir()
        .ok_or_else(|| format!("{ERR_STORE}: could not resolve the app data directory"))?;
    Ok(base.join("pet"))
}

pub fn load_prefs() -> PetPrefs {
    let Ok(dir) = prefs_dir() else {
        return PetPrefs::default();
    };
    match std::fs::read_to_string(dir.join(PREFS_FILE)) {
        Ok(raw) => prefs_from_str(&raw),
        Err(_) => PetPrefs::default(),
    }
}

/// Write through a temporary file and rename, like `profile::store` does: a
/// truncated `pet.json` would silently reset the corner on the next launch.
pub fn save_prefs(prefs: &PetPrefs) -> Result<(), String> {
    let dir = prefs_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{ERR_STORE}: {e}"))?;
    let body = serde_json::to_vec_pretty(prefs).map_err(|e| format!("{ERR_STORE}: {e}"))?;
    let tmp = dir.join(format!("{PREFS_FILE}.tmp"));
    std::fs::write(&tmp, &body).map_err(|e| format!("{ERR_STORE}: {e}"))?;
    std::fs::rename(&tmp, dir.join(PREFS_FILE)).map_err(|e| format!("{ERR_STORE}: {e}"))
}

// ---------------------------------------------------------------------------
// Intent
// ---------------------------------------------------------------------------

/// The seven closed animations of the Phase 5 contract plus the two moods that
/// have no art of their own, mapped to the atlas clip that plays them.
///
/// The mapping lives here, next to the window, rather than in `core::omni`:
/// `Animation` is a product concept and the atlas is an art file, and the two
/// are allowed to drift (Celebrate reuses `wave`, Worried reuses `idle`).
/// True while the floating pet window exists (the only consumer of the
/// external-agent watcher).
pub fn is_enabled(app: &AppHandle) -> bool {
    use tauri::Manager;
    app.get_webview_window(WINDOW_LABEL).is_some()
}

pub fn animation_to_clip(animation: &str) -> Option<&'static str> {
    Some(match animation {
        "Idle" | "idle" => "idle",
        "Walk" | "walk" => "walk",
        "Sit" | "sit" => "sit",
        "Wave" | "wave" => "wave",
        "Work" | "work" => "work",
        "Sleep" | "sleep" => "sleep",
        "Talk" | "talk" => "talk",
        "Celebrate" | "celebrate" => "wave",
        "Worried" | "worried" => "idle",
        _ => return None,
    })
}

pub const MOODS: [&str; 7] = [
    "Neutral", "Happy", "Curious", "Proud", "Worried", "Sleepy", "Focused",
];

fn normalize_mood(raw: Option<&str>) -> String {
    let Some(raw) = raw else {
        return "Neutral".to_string();
    };
    MOODS
        .iter()
        .find(|m| m.eq_ignore_ascii_case(raw))
        .map(|m| (*m).to_string())
        .unwrap_or_else(|| "Neutral".to_string())
}

/// Turn whatever arrived into the payload the pet window can draw, or reject
/// it. An unknown animation is an error (the caller has a closed enum and a
/// typo there is a bug); an unknown mood is not (it only tints a bubble).
///
/// `line` is trimmed, stripped of control characters and clipped to 80 *chars*
/// — not bytes, so a line of emoji is not cut in half.
pub fn normalize_intent(raw: &serde_json::Value) -> Result<serde_json::Value, String> {
    let obj = raw
        .as_object()
        .ok_or_else(|| format!("{ERR_INTENT}: intent is not an object"))?;
    let animation = obj
        .get("animation")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{ERR_INTENT}: intent has no animation"))?;
    let clip = animation_to_clip(animation)
        .ok_or_else(|| format!("{ERR_INTENT}: unknown animation {animation:?}"))?;
    let mood = normalize_mood(obj.get("mood").and_then(|v| v.as_str()));
    let line = obj.get("line").and_then(|v| v.as_str()).and_then(|s| {
        let clean: String = s
            .trim()
            .chars()
            .filter(|c| !c.is_control())
            .take(LINE_MAX_CHARS)
            .collect();
        let clean = clean.trim_end().to_string();
        if clean.is_empty() {
            None
        } else {
            Some(clean)
        }
    });
    Ok(json!({
        "animation": animation,
        "clip": clip,
        "mood": mood,
        "line": line,
    }))
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Transparency is a property of the compositor, not of the machine, and there
/// is no honest way to ask it from here: the only measurement available is
/// building the window and seeing whether it comes up. So `pet_capabilities`
/// reports what the platform guarantees and marks the rest "unknown" instead of
/// guessing, and `pet_open` is the one that can return `ERR_PET_UNSUPPORTED`.
fn transparency_confidence() -> &'static str {
    #[cfg(any(target_os = "macos", windows))]
    {
        "yes"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Wayland always composites. X11 needs a running compositing manager,
        // which this process cannot see without an X connection of its own.
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            "yes"
        } else {
            "unknown"
        }
    }
    #[cfg(not(any(target_os = "macos", windows, unix)))]
    {
        "unknown"
    }
}

fn capabilities_value(open: bool, prefs: &PetPrefs) -> serde_json::Value {
    let transparency = transparency_confidence();
    json!({
        "transparency": transparency,
        // A pet the cursor falls through is a nice-to-have; where the backend
        // refuses, the command errors and the toggle stays off.
        "click_through": cfg!(any(target_os = "macos", windows, all(unix, not(target_os = "macos")))),
        "always_on_top": true,
        // The UI offers the in-window pet instead of the floating one when the
        // compositor cannot be trusted with an alpha channel.
        "fallback_recommended": transparency != "yes",
        "open": open,
        "corner": prefs.corner.as_str(),
        "click_through_on": prefs.click_through,
        "size": [PET_W, PET_H],
    })
}

fn state_value(open: bool, prefs: &PetPrefs) -> serde_json::Value {
    json!({
        "open": open,
        "corner": prefs.corner.as_str(),
        "click_through": prefs.click_through,
        "enabled": prefs.enabled,
    })
}

/// Work area of the monitor the pet is on, in logical pixels.
fn work_area(window: &tauri::WebviewWindow) -> Result<WorkArea, String> {
    let monitor = window
        .current_monitor()
        .map_err(|e| format!("{ERR_WINDOW}: {e}"))?
        .or(window
            .primary_monitor()
            .map_err(|e| format!("{ERR_WINDOW}: {e}"))?)
        .ok_or_else(|| format!("{ERR_WINDOW}: no monitor"))?;
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    Ok(WorkArea {
        x: area.position.x as f64 / scale,
        y: area.position.y as f64 / scale,
        w: area.size.width as f64 / scale,
        h: area.size.height as f64 / scale,
    })
}

fn park(window: &tauri::WebviewWindow, corner: Corner) -> Result<(), String> {
    let area = work_area(window)?;
    let (x, y) = corner_position(corner, area, PET_W, PET_H, PET_MARGIN);
    window
        .set_position(tauri::LogicalPosition::new(x, y))
        .map_err(|e| format!("{ERR_WINDOW}: {e}"))
}

/// Open the pet window, or bring the existing one forward. Returns the state
/// the frontend needs to render its toggles.
#[tauri::command]
pub async fn pet_open(app: AppHandle) -> Result<serde_json::Value, String> {
    let mut prefs = load_prefs();
    prefs.enabled = true;
    // A page that is about to (re)mount has no questions queued yet.
    set_ask_pending(false);

    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.set_always_on_top(true);
        park(&window, prefs.corner)?;
        let _ = window.set_ignore_cursor_events(ignore_cursor_now(&prefs, ask_pending()));
        save_prefs(&prefs)?;
        return Ok(state_value(true, &prefs));
    }

    let window = crate::core::portable::with_dev_webview_dir(WebviewWindowBuilder::new(
        &app,
        WINDOW_LABEL,
        WebviewUrl::App("/pet".into()),
    ))
    .title("Omni")
    .inner_size(PET_W, PET_H)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible_on_all_workspaces(true)
    // Without this the first click on macOS only focuses the window, so
    // dragging the pet would always take two gestures.
    .accept_first_mouse(true)
    .focused(false)
    .build()
    // A compositor with no ARGB visual is the common reason this fails, and
    // it is exactly the case the in-app fallback exists for.
    .map_err(|e| format!("{ERR_UNSUPPORTED}: {e}"))?;

    park(&window, prefs.corner)?;
    if prefs.click_through {
        let _ = window.set_ignore_cursor_events(true);
    }
    save_prefs(&prefs)?;
    Ok(state_value(true, &prefs))
}

#[tauri::command]
pub async fn pet_close(app: AppHandle) -> Result<serde_json::Value, String> {
    let mut prefs = load_prefs();
    prefs.enabled = false;
    set_ask_pending(false);
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        window.close().map_err(|e| format!("{ERR_WINDOW}: {e}"))?;
    }
    save_prefs(&prefs)?;
    Ok(state_value(false, &prefs))
}

#[tauri::command]
pub async fn pet_set_corner(app: AppHandle, corner: String) -> Result<serde_json::Value, String> {
    let parsed = parse_corner(&corner)?;
    let mut prefs = load_prefs();
    prefs.corner = parsed;
    save_prefs(&prefs)?;
    let open = match app.get_webview_window(WINDOW_LABEL) {
        Some(window) => {
            park(&window, parsed)?;
            true
        }
        None => false,
    };
    Ok(state_value(open, &prefs))
}

#[tauri::command]
pub async fn pet_set_click_through(
    app: AppHandle,
    enabled: bool,
) -> Result<serde_json::Value, String> {
    let mut prefs = load_prefs();
    prefs.click_through = enabled;
    let window = app
        .get_webview_window(WINDOW_LABEL)
        .ok_or_else(|| format!("{ERR_NOT_OPEN}: the pet window is closed"))?;
    // The preference is stored as asked, but a question on screen still wins:
    // turning click-through on while the pet is waiting for an allow/deny would
    // hide two buttons under the cursor. It takes effect when the ask is over.
    window
        .set_ignore_cursor_events(ignore_cursor_now(&prefs, ask_pending()))
        .map_err(|e| format!("{ERR_WINDOW}: {e}"))?;
    save_prefs(&prefs)?;
    Ok(state_value(true, &prefs))
}

/// Whether the pet window should let the cursor fall through *right now*.
///
/// Click-through is a preference, but a pending tool permission is a question
/// with two buttons on it: while one is up the window has to catch clicks, or
/// the user would be looking at a dialog the cursor goes straight past. The
/// preference itself is never rewritten — this is the effective value, and the
/// window goes back to the stored one the moment the queue empties.
///
/// Pure, so the one rule that matters ("an ask always wins over the pref") is
/// covered by a test instead of by a screenshot on somebody's desktop.
pub fn ignore_cursor_now(prefs: &PetPrefs, ask_pending: bool) -> bool {
    prefs.click_through && !ask_pending
}

/// Whether a tool permission is on screen right now. Not a preference and not
/// persisted: it is the pet page's queue, mirrored here so `pet_set_click_through`
/// cannot hand the user a toggle that hides the buttons they are looking at.
/// Reset when the window opens or closes, because a fresh page starts with an
/// empty queue (it only hears the asks emitted after it attached its listener).
static ASK_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn ask_pending() -> bool {
    ASK_PENDING.load(std::sync::atomic::Ordering::Relaxed)
}

fn set_ask_pending(value: bool) {
    ASK_PENDING.store(value, std::sync::atomic::Ordering::Relaxed);
}

/// Tell the window whether a tool permission is waiting for an answer.
///
/// Called by the pet page when its FIFO of `llm://tool-ask` questions goes from
/// empty to non-empty and back — event-driven, never polled, so an idle pet
/// still owns no timer on either side of the bridge.
///
/// A closed window is not an error: the page calls this while tearing down, and
/// a rejected promise there would only be noise in the console.
#[tauri::command]
pub async fn pet_set_ask_pending(
    app: AppHandle,
    pending: bool,
) -> Result<serde_json::Value, String> {
    let prefs = load_prefs();
    set_ask_pending(pending);
    let ignore = ignore_cursor_now(&prefs, pending);
    let open = match app.get_webview_window(WINDOW_LABEL) {
        Some(window) => {
            window
                .set_ignore_cursor_events(ignore)
                .map_err(|e| format!("{ERR_WINDOW}: {e}"))?;
            // A question the user cannot see is a turn that hangs until the
            // broker's ask timeout, so the pet comes back to the front.
            if pending {
                let _ = window.show();
                let _ = window.set_always_on_top(true);
            }
            true
        }
        None => false,
    };
    Ok(json!({
        "open": open,
        "ask_pending": pending,
        // What the window is doing now, and what it will go back to.
        "click_through": ignore,
        "click_through_pref": prefs.click_through,
    }))
}

#[tauri::command]
pub async fn pet_capabilities(app: AppHandle) -> Result<serde_json::Value, String> {
    let prefs = load_prefs();
    let open = app.get_webview_window(WINDOW_LABEL).is_some();
    Ok(capabilities_value(open, &prefs))
}
