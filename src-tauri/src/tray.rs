use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

use tauri::{
    menu::{MenuBuilder, MenuItem, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Manager, Wry,
};

static DOWNLOADS_ITEM: OnceLock<MenuItem<Wry>> = OnceLock::new();
static LAST_ACTIVE: AtomicU32 = AtomicU32::new(0);

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let open_item = MenuItemBuilder::with_id("open", "OmniGet").build(app)?;
    let downloads_item =
        MenuItemBuilder::with_id("downloads", active_label(0))
            .enabled(false)
            .build(app)?;
    DOWNLOADS_ITEM.set(downloads_item.clone()).ok();
    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&open_item)
        .separator()
        .item(&downloads_item)
        .separator()
        .item(&quit_item)
        .build()?;

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("app icon not found");

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("OmniGet")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_window(app),
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

pub fn update_active_count(_app: &AppHandle, count: u32) {
    if let Some(item) = DOWNLOADS_ITEM.get() {
        let _ = item.set_text(active_label(count));
    }
}

pub fn compute_total_active(app: &AppHandle) -> u32 {
    let state = app.state::<crate::AppState>();

    let queue_count = match state.download_queue.try_lock() {
        Ok(q) => q.active_count(),
        Err(_) => return LAST_ACTIVE.load(Ordering::Relaxed),
    };

    let tg_count = match state.active_generic_downloads.try_lock() {
        Ok(active) => active
            .values()
            .filter(|(key, _)| key.starts_with("tg-batch:"))
            .count() as u32,
        Err(_) => return LAST_ACTIVE.load(Ordering::Relaxed),
    };

    let total = queue_count + tg_count;
    LAST_ACTIVE.store(total, Ordering::Relaxed);
    total
}

fn active_label(count: u32) -> String {
    if count == 0 {
        "No active downloads".into()
    } else {
        format!("Downloads: {} active", count)
    }
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
