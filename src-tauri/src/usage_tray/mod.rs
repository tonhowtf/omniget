//! The usage icon in the menu bar: a second status item, separate from the
//! app's own tray icon (`crate::tray`, id `main-tray`), that shows how much of
//! the Claude Code / Codex limits is used.
//!
//! Reference: Claude-Usage-Tracker (github.com/hamed-elfayome/Claude-Usage-Tracker,
//! MIT), `Claude Usage/MenuBar/MenuBarManager.swift`:
//!   - a left click toggles a popover anchored under the status item
//!     (`togglePopover`, ~L523; `showPopover`, ~L614) and a click anywhere
//!     else closes it (`startMonitoringForOutsideClicks`, ~L670); a click on
//!     the icon that lands right after that close is the dismissing click and
//!     does not reopen it (~L590, 0.25 s);
//!   - a right click opens a short native menu instead (`showContextMenu`,
//!     ~L630).
//!
//! Here the popover is a small undecorated webview (`/usage-panel`) that hides
//! when it loses focus, and the context menu is the status item's own menu
//! (`show_menu_on_left_click(false)`). The ring glyph and the forecast are in
//! `icon.rs` and `view.rs`, with their own references.
//!
//! Data: the limits strip's engine (`limits_strip::engine`), which already
//! reads every Claude and Codex login. `engine::set_tray` keeps it running
//! while this icon is on and adds the rings listed here; `engine` calls
//! [`on_state`] whenever those rings change. No reader is duplicated and the
//! strip's privacy rules hold: a ring is read only once the user lists it.

pub mod icon;
pub mod prefs;
pub mod spend;
pub mod view;

use prefs::{Shows, TrayPrefs};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use tauri::image::Image;
use tauri::menu::{CheckMenuItemBuilder, Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, Rect, WebviewUrl, WebviewWindowBuilder, Wry,
};

use crate::limits_strip::engine::{self, Snapshot};
use crate::limits_strip::{now_ms, providers};

pub const TRAY_ID: &str = "usage-tray";
pub const PANEL_LABEL: &str = "usage-panel";
pub const EVENT_VIEW: &str = "usage-tray://view";
/// Menu ids carry this prefix: the tray menu handler Tauri calls is global,
/// so `crate::tray` routes them here and the main tray never sees a clash.
pub const MENU_PREFIX: &str = "usage-tray:";

const PANEL_W: f64 = 340.0;
const PANEL_H: f64 = 420.0;
/// A click on the icon this soon after the panel hid on blur is the click
/// that hid it (Claude-Usage-Tracker uses 0.25 s).
const REOPEN_GUARD_MS: i64 = 300;
const THRESHOLDS: [(u8, u8); 3] = [(70, 90), (80, 95), (90, 98)];

#[derive(Default)]
struct State {
    prefs: TrayPrefs,
    strings: HashMap<String, String>,
    view: Option<view::View>,
    hidden_at: i64,
    /// Where the icon was on the last click, to re-anchor after a resize.
    anchor: Option<Rect>,
    /// Ids the menu was last built with, to rebuild it when accounts change.
    menu_ids: Vec<String>,
    /// What the status item shows now. Every `set_icon` / `set_title` /
    /// `set_tooltip` waits for the main thread, so an unchanged look is not
    /// sent again.
    painted: Option<Painted>,
}

#[derive(Debug, Clone, PartialEq)]
struct Painted {
    /// Whole percent: the ring cannot show less, and a 0.1 % drift would
    /// repaint on every read.
    used: Option<i64>,
    level: &'static str,
    colored: bool,
    title: Option<String>,
    tooltip: String,
}

fn state() -> MutexGuard<'static, State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Serialises every change of the prefs from read to publish, so two changes
/// at once (a menu click and a panel switch) cannot both start from the same
/// copy and the second one silently drop the first. Separate from [`state`]
/// so readers never wait for a file write.
fn writer() -> MutexGuard<'static, ()> {
    static W: OnceLock<Mutex<()>> = OnceLock::new();
    W.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Applies `change` to a copy of the prefs, has `save` write it, and only then
/// makes it the prefs in memory. A save that fails leaves memory as the disk
/// has it and answers the error, so nothing shows a choice that a restart
/// would lose. Returns the prefs before and after.
fn update_prefs(
    change: impl FnOnce(&mut TrayPrefs),
    save: impl FnOnce(&TrayPrefs) -> Result<(), String>,
) -> Result<(TrayPrefs, TrayPrefs), String> {
    let _writing = writer();
    let before = state().prefs.clone();
    let mut next = before.clone();
    change(&mut next);
    let next = next.sane();
    save(&next)?;
    state().prefs = next.clone();
    Ok((before, next))
}

// ---------------------------------------------------------------------------
// Strings: the menu is native, so the frontend pushes the translations
// (same pattern as `sync_tray_strings`). English is the fallback.
// ---------------------------------------------------------------------------

fn english(key: &str) -> &'static str {
    match key {
        "accounts" => "Accounts shown",
        "no_accounts" => "No Claude Code or Codex login found",
        "icon_shows" => "Icon shows",
        "shows_session" => "Session (5 h)",
        "shows_weekly" => "Weekly",
        "shows_busiest" => "Most used account",
        "percent_text" => "Show percentage",
        "colored" => "Loop in colour",
        "thresholds" => "Alert thresholds",
        "threshold_item" => "Orange at {{warn}}%, red at {{critical}}%",
        "open_claude" => "Open Claude Code in this account",
        "add_account" => "Add account…",
        "open_app" => "Open OmniGet",
        "refresh" => "Refresh now",
        "hide" => "Hide this icon",
        "tooltip" => "OmniGet usage",
        "toggle" => "Usage icon in the menu bar",
        "default_login" => "Terminal login",
        "session" => "session",
        "weekly" => "weekly",
        _ => "",
    }
}

fn s(key: &str) -> String {
    state()
        .strings
        .get(key)
        .filter(|v| !v.is_empty())
        .cloned()
        .unwrap_or_else(|| english(key).to_string())
}

// ---------------------------------------------------------------------------
// Accounts the menu offers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CatalogItem {
    pub id: String,
    pub cli: String,
    pub name: String,
    pub shown: bool,
}

/// Every Claude Code / Codex login OmniGet knows (offline: `accounts.json`
/// and each profile's `.claude.json`, no credential).
fn catalog(prefs: &TrayPrefs) -> Vec<CatalogItem> {
    let default_login = s("default_login");
    providers::cached()
        .iter()
        .filter(|p| prefs::is_cli_ring(p.id()))
        .map(|p| {
            let tag = p.account().cloned().unwrap_or_default();
            let who = tag.label.or(tag.email);
            let name = match who {
                Some(w) => format!("{} · {}", p.label(), w),
                None => format!("{} · {}", p.label(), default_login),
            };
            CatalogItem {
                id: p.id().to_string(),
                cli: p.id().split(':').next().unwrap_or("").to_string(),
                name,
                shown: prefs.accounts.iter().any(|a| a == p.id()),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Setup, enabling, the menu
// ---------------------------------------------------------------------------

/// Called once from the app's setup, after the main tray exists.
pub fn setup(app: &AppHandle) {
    let loaded = prefs::load();
    let mut p = loaded.clone();
    if p.seed_from(&crate::limits_strip::prefs::load()) {
        // Seeded only once it is on disk; otherwise keep what the file says
        // and seed again on the next launch.
        if let Err(e) = prefs::save(&p) {
            tracing::warn!("usage tray: could not save the seeded accounts: {e}");
            p = loaded;
        }
    }
    let on = p.enabled;
    state().prefs = p;
    crate::tray::set_usage_toggle(on);
    if on {
        apply_enabled(app, true);
    }
}

fn apply_enabled(app: &AppHandle, on: bool) {
    if on {
        if app.tray_by_id(TRAY_ID).is_none() {
            if let Err(e) = build_tray(app) {
                tracing::warn!("usage tray icon: {e}");
                return;
            }
        }
        let ids = state().prefs.accounts.clone();
        engine::set_tray(app, true, ids);
        republish(app);
    } else {
        close_panel(app);
        let _ = app.remove_tray_by_id(TRAY_ID);
        state().painted = None;
        engine::set_tray(app, false, Vec::new());
    }
    crate::tray::set_usage_toggle(on);
}

pub fn set_enabled(app: &AppHandle, on: bool) -> Result<(), String> {
    commit(app, |p| p.enabled = on)
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    state().painted = None;
    let (px, template) = icon::render(icon::Look {
        used: None,
        level: "ok",
        colored: state().prefs.colored,
    });
    let menu = build_menu(app)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::new_owned(px, icon::SIZE, icon::SIZE))
        .icon_as_template(template)
        .tooltip(s("tooltip"))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                let app = tray.app_handle().clone();
                // Never build a window inside the tray callback (it deadlocks
                // on Windows); hop onto the runtime.
                tauri::async_runtime::spawn(async move {
                    toggle_panel(&app, rect);
                });
            }
        })
        .build(app)?;
    Ok(())
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let p = state().prefs.clone();
    let items = catalog(&p);
    state().menu_ids = items.iter().map(|i| i.id.clone()).collect();

    let mut accounts = SubmenuBuilder::new(app, s("accounts"));
    if items.is_empty() {
        accounts = accounts.item(
            &MenuItemBuilder::with_id(format!("{MENU_PREFIX}none"), s("no_accounts"))
                .enabled(false)
                .build(app)?,
        );
    }
    for i in &items {
        accounts = accounts.item(
            &CheckMenuItemBuilder::with_id(format!("{MENU_PREFIX}acct:{}", i.id), &i.name)
                .checked(i.shown)
                .build(app)?,
        );
    }

    let mut shows = SubmenuBuilder::new(app, s("icon_shows"));
    for (v, key) in [
        (Shows::Session, "shows_session"),
        (Shows::Weekly, "shows_weekly"),
        (Shows::Busiest, "shows_busiest"),
    ] {
        shows = shows.item(
            &CheckMenuItemBuilder::with_id(format!("{MENU_PREFIX}shows:{}", v.as_str()), s(key))
                .checked(p.shows == v)
                .build(app)?,
        );
    }
    shows = shows
        .separator()
        .item(
            &CheckMenuItemBuilder::with_id(format!("{MENU_PREFIX}pct"), s("percent_text"))
                .checked(p.percent_text)
                .build(app)?,
        )
        .item(
            &CheckMenuItemBuilder::with_id(format!("{MENU_PREFIX}color"), s("colored"))
                .checked(p.colored)
                .build(app)?,
        );

    let mut thr = SubmenuBuilder::new(app, s("thresholds"));
    for (w, c) in THRESHOLDS {
        let label = s("threshold_item")
            .replace("{{warn}}", &w.to_string())
            .replace("{{critical}}", &c.to_string());
        thr = thr.item(
            &CheckMenuItemBuilder::with_id(format!("{MENU_PREFIX}thr:{w}-{c}"), label)
                .checked(p.warn == w && p.critical == c)
                .build(app)?,
        );
    }

    let claude: Vec<&CatalogItem> = items.iter().filter(|i| i.cli == "claude").collect();
    let mut open = SubmenuBuilder::new(app, s("open_claude"));
    for i in &claude {
        open = open.item(
            &MenuItemBuilder::with_id(format!("{MENU_PREFIX}claude:{}", i.id), &i.name)
                .build(app)?,
        );
    }
    if !claude.is_empty() {
        open = open.separator();
    }
    open = open
        .item(&MenuItemBuilder::with_id(format!("{MENU_PREFIX}add"), s("add_account")).build(app)?);

    MenuBuilder::new(app)
        .item(&accounts.build()?)
        .item(&shows.build()?)
        .item(&thr.build()?)
        .separator()
        .item(&open.build()?)
        .separator()
        .item(&MenuItemBuilder::with_id(format!("{MENU_PREFIX}refresh"), s("refresh")).build(app)?)
        .item(&MenuItemBuilder::with_id(format!("{MENU_PREFIX}open"), s("open_app")).build(app)?)
        .item(&MenuItemBuilder::with_id(format!("{MENU_PREFIX}hide"), s("hide")).build(app)?)
        .build()
}

fn rebuild_menu(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        match build_menu(app) {
            Ok(menu) => {
                let _ = tray.set_menu(Some(menu));
            }
            Err(e) => tracing::warn!("usage tray menu: {e}"),
        }
    }
}

/// Saves the prefs, then tells the engine which rings to read and redraws.
/// A failed save changes nothing at runtime: the menu's check mark is put back
/// to what is saved and the error goes to the caller.
fn commit(app: &AppHandle, change: impl FnOnce(&mut TrayPrefs)) -> Result<(), String> {
    let (before, p) = match update_prefs(change, prefs::save) {
        Ok(done) => done,
        Err(e) => {
            crate::tray::set_usage_toggle(state().prefs.enabled);
            return Err(e);
        }
    };
    if p.enabled != before.enabled {
        apply_enabled(app, p.enabled);
    } else if p.enabled {
        engine::set_tray(app, true, p.accounts.clone());
    }
    rebuild_menu(app);
    republish(app);
    Ok(())
}

/// Every menu id with [`MENU_PREFIX`] lands here, from `crate::tray`.
///
/// Menu events arrive on the main thread; what they trigger saves prefs,
/// looks up accounts and redraws the status item, so it runs on the async
/// runtime and the menu bar never waits for it.
pub fn on_menu(app: &AppHandle, id: &str) {
    if !id.starts_with(MENU_PREFIX) {
        return;
    }
    let (app, id) = (app.clone(), id.to_string());
    tauri::async_runtime::spawn(async move {
        handle_menu(&app, &id);
    });
}

fn handle_menu(app: &AppHandle, id: &str) {
    let Some(rest) = id.strip_prefix(MENU_PREFIX) else {
        return;
    };
    // A menu click has nobody to answer: a save that failed is logged, and
    // `commit` already left memory and the menu as they were.
    let saved = match rest {
        "toggle" => commit(app, |p| p.enabled = !p.enabled),
        "hide" => set_enabled(app, false),
        "open" => {
            crate::tray::show_window(app);
            Ok(())
        }
        "refresh" => {
            refresh(app);
            Ok(())
        }
        "pct" => commit(app, |p| p.percent_text = !p.percent_text),
        "color" => commit(app, |p| p.colored = !p.colored),
        "add" => {
            if let Err(e) = open_terminal(None, true) {
                tracing::warn!("usage tray: {e}");
            }
            Ok(())
        }
        other => {
            if let Some(acct) = other.strip_prefix("acct:") {
                commit(app, |p| p.toggle_account(acct))
            } else if let Some(v) = other.strip_prefix("shows:").and_then(Shows::parse) {
                commit(app, |p| p.shows = v)
            } else if let Some((w, c)) = other
                .strip_prefix("thr:")
                .and_then(|t| t.split_once('-'))
                .and_then(|(w, c)| Some((w.parse::<u8>().ok()?, c.parse::<u8>().ok()?)))
            {
                commit(app, |p| {
                    p.warn = w;
                    p.critical = c;
                })
            } else {
                if let Some(ring) = other.strip_prefix("claude:") {
                    if let Err(e) = open_terminal(Some(ring), false) {
                        tracing::warn!("usage tray: {e}");
                    }
                }
                Ok(())
            }
        }
    };
    if let Err(e) = saved {
        tracing::warn!("usage tray: the change was not saved: {e}");
    }
}

fn refresh(app: &AppHandle) -> Vec<String> {
    let ids = state().prefs.accounts.clone();
    let mut out = Vec::new();
    for id in ids {
        out.extend(engine::refresh(Some(&id)));
    }
    engine::wake();
    spend::refresh_if_stale(app);
    out
}

// ---------------------------------------------------------------------------
// State from the engine → icon, tooltip, panel
// ---------------------------------------------------------------------------

/// Called by the engine when the rings this icon lists change.
pub fn on_state(app: &AppHandle, snap: &Snapshot) {
    let (p, stale_menu) = {
        let g = state();
        (g.prefs.clone(), g.menu_ids.clone())
    };
    let mut v = view::build(snap, &p, now_ms());
    let spent = spend::cached();
    for a in &mut v.accounts {
        a.spend_today = spent.get(&a.id).copied();
    }
    paint(app, &v, &p);
    state().view = Some(v.clone());
    let _ = app.emit_to(PANEL_LABEL, EVENT_VIEW, &v);
    // A login added or removed in the app shows up in the menu.
    let ids: Vec<String> = catalog(&p).into_iter().map(|i| i.id).collect();
    if ids != stale_menu {
        rebuild_menu(app);
    }
}

/// Redraws from the engine's current rings (prefs changed, spend landed).
pub fn republish(app: &AppHandle) {
    if !state().prefs.enabled {
        return;
    }
    let snap = engine::tray_snapshot(app);
    on_state(app, &snap);
}

fn paint(app: &AppHandle, v: &view::View, p: &TrayPrefs) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let (used, level) = match v.headline {
        Some((pct, lvl)) => (Some(pct.round() as i64), lvl),
        None => (None, "ok"),
    };
    let title = if p.percent_text {
        icon::title(v.headline.map(|h| h.0))
    } else {
        None
    };
    let next = Painted {
        used,
        level,
        colored: p.colored,
        title,
        tooltip: tooltip(v),
    };
    let before = state().painted.clone();
    let look_changed = before
        .as_ref()
        .is_none_or(|b| (b.used, b.level, b.colored) != (next.used, next.level, next.colored));
    if look_changed {
        let (px, template) = icon::render(icon::Look {
            used: next.used.map(|u| u as f64 / 100.0),
            level,
            colored: p.colored,
        });
        let _ = tray.set_icon(Some(Image::new_owned(px, icon::SIZE, icon::SIZE)));
        let _ = tray.set_icon_as_template(template);
    }
    if before.as_ref().is_none_or(|b| b.title != next.title) {
        let _ = tray.set_title(next.title.clone());
    }
    if before.as_ref().is_none_or(|b| b.tooltip != next.tooltip) {
        let _ = tray.set_tooltip(Some(next.tooltip.clone()));
    }
    state().painted = Some(next);
}

fn tooltip(v: &view::View) -> String {
    let mut lines = vec![s("tooltip")];
    let pct = |w: &Option<view::Win>| w.as_ref().map(|w| format!("{:.0}%", w.used * 100.0));
    for a in &v.accounts {
        let parts: Vec<String> = [
            pct(&a.session).map(|x| format!("{} {x}", s("session"))),
            pct(&a.weekly).map(|x| format!("{} {x}", s("weekly"))),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !parts.is_empty() {
            lines.push(format!("{}: {}", a.name, parts.join(" · ")));
        }
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// The panel (left click)
// ---------------------------------------------------------------------------

fn toggle_panel(app: &AppHandle, rect: Rect) {
    if let Some(w) = app.get_webview_window(PANEL_LABEL) {
        if w.is_visible().unwrap_or(false) {
            hide_panel(app);
            return;
        }
    }
    if now_ms() - state().hidden_at < REOPEN_GUARD_MS {
        return;
    }
    show_panel(app, Some(rect));
}

/// Shows the panel under `rect` (the icon), or under the icon's current
/// place when no click told us where it is.
pub(crate) fn show_panel(app: &AppHandle, rect: Option<Rect>) {
    let rect = rect.or_else(|| {
        app.tray_by_id(TRAY_ID)
            .and_then(|t| t.rect().ok().flatten())
    });
    if rect.is_some() {
        state().anchor = rect;
    }
    let window = match app.get_webview_window(PANEL_LABEL) {
        Some(w) => w,
        None => match crate::core::portable::with_dev_webview_dir(WebviewWindowBuilder::new(
            app,
            PANEL_LABEL,
            WebviewUrl::App("/usage-panel".into()),
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
                        hide_panel(&handle);
                    }
                });
                w
            }
            Err(e) => {
                tracing::warn!("usage panel: {e}");
                return;
            }
        },
    };
    place(&window, PANEL_H);
    let _ = window.show();
    let _ = window.set_focus();
    spend::refresh_if_stale(app);
    if let Some(v) = state().view.clone() {
        let _ = app.emit_to(PANEL_LABEL, EVENT_VIEW, &v);
    }
}

/// Under the icon (or above it when the icon sits at the bottom of the
/// screen, as on a Windows taskbar), kept inside the monitor.
fn place(window: &tauri::WebviewWindow, height: f64) {
    let Some(rect) = state().anchor else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    let pos = rect.position.to_physical::<f64>(scale);
    let size = rect.size.to_physical::<f64>(scale);
    let (w, h) = (PANEL_W * scale, height * scale);
    let gap = 6.0 * scale;
    let mut x = pos.x + size.width / 2.0 - w / 2.0;
    let mut y = pos.y + size.height + gap;
    if let Ok(Some(m)) = window.monitor_from_point(pos.x, pos.y) {
        let (mx, my) = (m.position().x as f64, m.position().y as f64);
        let (mw, mh) = (m.size().width as f64, m.size().height as f64);
        if pos.y > my + mh / 2.0 {
            y = pos.y - h - gap;
        }
        x = x.clamp(mx + gap, (mx + mw - w - gap).max(mx + gap));
        y = y.clamp(my, (my + mh - h).max(my));
    }
    let _ = window.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

fn hide_panel(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(PANEL_LABEL) {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
            state().hidden_at = now_ms();
        }
    }
}

fn close_panel(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(PANEL_LABEL) {
        let _ = w.close();
    }
}

// ---------------------------------------------------------------------------
// Terminal launchers
// ---------------------------------------------------------------------------

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The shell line: `omniget claude <account>` for an app account, plain
/// `claude` for the terminal's own login, `omniget claude --add` to add one.
pub fn launch_line(ring: Option<&str>, add: bool) -> String {
    if add {
        return "omniget claude --add".to_string();
    }
    match ring.and_then(|r| r.strip_prefix("claude:")) {
        Some(acct) => format!("omniget claude {}", sh_quote(acct)),
        None => "claude".to_string(),
    }
}

/// Opens a terminal window running [`launch_line`]. On macOS a `.command`
/// file opens in Terminal.app, as `/llm → Accounts` already does for logins.
fn open_terminal(ring: Option<&str>, add: bool) -> Result<(), String> {
    let line = launch_line(ring, add);
    #[cfg(windows)]
    {
        crate::core::process::std_command("cmd")
            .args(["/C", "start", "", "cmd", "/K", &line])
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        let dir = crate::core::paths::app_data_dir()
            .ok_or("no data directory")?
            .join("llm")
            .join("launchers");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let ext = if cfg!(target_os = "macos") {
            "command"
        } else {
            "sh"
        };
        let path = dir.join(format!("usage-tray-claude.{ext}"));
        let script = format!(
            "#!/bin/sh\n\
             # Opened by OmniGet's usage icon.\n\
             export PATH=\"/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$HOME/.cargo/bin:$PATH\"\n\
             if ! command -v {bin} >/dev/null 2>&1; then\n\
             \x20 echo \"{bin}: not found in PATH\"; exec \"${{SHELL:-/bin/sh}}\" -l\n\
             fi\n\
             {line}\n\
             exec \"${{SHELL:-/bin/sh}}\" -l\n",
            bin = line.split(' ').next().unwrap_or("omniget"),
        );
        std::fs::write(&path, script).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }
        let path = path.to_string_lossy().to_string();
        let mut cmd = if cfg!(target_os = "macos") {
            let mut c = crate::core::process::std_command("open");
            c.args(["-a", "Terminal", &path]);
            c
        } else {
            let mut c = crate::core::process::std_command("x-terminal-emulator");
            c.args(["-e", "sh", &path]);
            c
        };
        cmd.spawn().map_err(|e| e.to_string())?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Commands for the panel and the main window
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct Panel {
    pub view: Option<view::View>,
    pub catalog: Vec<CatalogItem>,
    pub prefs: TrayPrefs,
}

#[tauri::command]
pub async fn usage_tray_view(app: AppHandle) -> Result<Panel, String> {
    if state().view.is_none() {
        republish(&app);
    }
    let p = state().prefs.clone();
    Ok(panel_snapshot(p))
}

fn panel_snapshot(p: TrayPrefs) -> Panel {
    // `catalog` reads translated strings under the same mutex. Release the
    // view's temporary guard before evaluating the next struct field.
    let view = state().view.clone();
    let catalog = catalog(&p);
    Panel {
        view,
        catalog,
        prefs: p,
    }
}

#[tauri::command]
pub async fn usage_tray_toggle_account(app: AppHandle, id: String) -> Result<(), String> {
    commit(&app, |p| p.toggle_account(&id))
}

/// `enabled`, `shows`, `percent_text`, `colored`, `warn`, `critical`.
/// One save for the whole patch; when it fails the answer is the error and the
/// prefs in memory, the icon and the menu stay as they were.
#[tauri::command]
pub async fn usage_tray_set(app: AppHandle, patch: serde_json::Value) -> Result<TrayPrefs, String> {
    commit(&app, |p| apply_patch(p, &patch))?;
    Ok(state().prefs.clone())
}

/// The fields `usage_tray_set` accepts; anything else in `patch` is ignored.
fn apply_patch(p: &mut TrayPrefs, patch: &serde_json::Value) {
    if let Some(v) = patch
        .get("shows")
        .and_then(|v| v.as_str())
        .and_then(Shows::parse)
    {
        p.shows = v;
    }
    if let Some(v) = patch.get("percent_text").and_then(|v| v.as_bool()) {
        p.percent_text = v;
    }
    if let Some(v) = patch.get("colored").and_then(|v| v.as_bool()) {
        p.colored = v;
    }
    if let Some(v) = patch.get("warn").and_then(|v| v.as_u64()) {
        p.warn = v.min(100) as u8;
    }
    if let Some(v) = patch.get("critical").and_then(|v| v.as_u64()) {
        p.critical = v.min(100) as u8;
    }
    if let Some(v) = patch.get("enabled").and_then(|v| v.as_bool()) {
        p.enabled = v;
    }
}

#[tauri::command]
pub async fn usage_tray_refresh(app: AppHandle) -> Result<Vec<String>, String> {
    Ok(refresh(&app))
}

#[tauri::command]
pub async fn usage_tray_open_claude(id: Option<String>, add: bool) -> Result<(), String> {
    open_terminal(id.as_deref(), add)
}

#[tauri::command]
pub async fn usage_tray_open_app(app: AppHandle) -> Result<(), String> {
    hide_panel(&app);
    crate::tray::show_window(&app);
    Ok(())
}

/// The panel measured its content: grow or shrink the window, keep it anchored.
#[tauri::command]
pub async fn usage_tray_resize(app: AppHandle, height: f64) -> Result<(), String> {
    let Some(w) = app.get_webview_window(PANEL_LABEL) else {
        return Ok(());
    };
    let h = height.clamp(120.0, 640.0);
    let _ = w.set_size(tauri::LogicalSize::new(PANEL_W, h));
    place(&w, h);
    Ok(())
}

#[tauri::command]
pub async fn usage_tray_close_panel(app: AppHandle) -> Result<(), String> {
    hide_panel(&app);
    Ok(())
}

/// The frontend's translations for the native menu and the tooltip.
#[tauri::command]
pub async fn usage_tray_sync_strings(
    app: AppHandle,
    strings: HashMap<String, String>,
) -> Result<(), String> {
    if state().strings == strings {
        return Ok(());
    }
    state().strings = strings;
    crate::tray::set_usage_toggle_text(&s("toggle"));
    rebuild_menu(&app);
    republish(&app);
    Ok(())
}
