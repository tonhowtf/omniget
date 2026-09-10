use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItem, MenuItemBuilder, Submenu, SubmenuBuilder},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, Wry,
};

static DOWNLOADS_ITEM: OnceLock<MenuItem<Wry>> = OnceLock::new();
static CHANNELS_SUBMENU: OnceLock<Submenu<Wry>> = OnceLock::new();
static BASE_ICON: OnceLock<(Vec<u8>, u32, u32)> = OnceLock::new();
static LAST_ACTIVE: AtomicU32 = AtomicU32::new(0);
static ICON_COUNT: AtomicU32 = AtomicU32::new(0);
static LAST_TOOLTIP_MS: AtomicU64 = AtomicU64::new(0);
static LAST_SPEED_BUCKET: AtomicU64 = AtomicU64::new(u64::MAX);

const SPEED_TOOLTIP_MIN_INTERVAL_MS: u64 = 2000;
static BADGE_CACHE: OnceLock<Mutex<BadgeCache>> = OnceLock::new();
static UI_LANG: OnceLock<String> = OnceLock::new();

/// Tray menus are native — `$t` is unreachable here. Resolve the UI language
/// once from settings.json (same store the settings manager persists) so the
/// tray can speak the user's language without frontend round-trips.
fn ui_lang(app: &AppHandle) -> &'static str {
    UI_LANG
        .get_or_init(|| {
            crate::core::paths::app_data_dir()
                .and_then(|dir| std::fs::read_to_string(dir.join("settings.json")).ok())
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|v| {
                    v.get("app_settings")?
                        .get("appearance")?
                        .get("language")?
                        .as_str()
                        .map(|s| s.to_string())
                })
                .unwrap_or_default()
        })
        .as_str()
}

fn tr<'a>(lang: &str, en: &'a str, ru: &'a str) -> &'a str {
    if lang == "ru" {
        ru
    } else {
        en
    }
}

struct BadgeCache {
    cache: HashMap<(u32, u32, u32), Vec<u8>>,
    icon_base_hash: u64,
}

impl BadgeCache {
    fn new() -> Self {
        Self {
            cache: HashMap::new(),
            icon_base_hash: 0,
        }
    }

    fn get_or_render(&mut self, base: &[u8], w: u32, h: u32, count: u32) -> Vec<u8> {
        let mut hasher = DefaultHasher::new();
        base.hash(&mut hasher);
        let base_hash = hasher.finish();

        if self.icon_base_hash != base_hash {
            self.cache.clear();
            self.icon_base_hash = base_hash;
        }

        let key = (w, h, count);
        if let Some(cached) = self.cache.get(&key) {
            cached.clone()
        } else {
            let rendered = render_badge(base, w, h, count);
            self.cache.insert(key, rendered.clone());
            rendered
        }
    }
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let lang = ui_lang(app);
    let open_item = MenuItemBuilder::with_id("open", "OmniGet").build(app)?;
    let downloads_item = MenuItemBuilder::with_id("downloads", active_label(lang, 0))
        .enabled(false)
        .build(app)?;
    DOWNLOADS_ITEM.set(downloads_item.clone()).ok();
    let quit_item = MenuItemBuilder::with_id("quit", tr(lang, "Quit", "Выход")).build(app)?;

    // Empty, hidden until the frontend pushes localized channel labels via
    // sync_channels_tray (the tray menu is native — $t is not reachable here).
    let channels_submenu = SubmenuBuilder::new(app, tr(lang, "Channels", "Каналы")).build()?;
    channels_submenu.set_enabled(false).ok();
    CHANNELS_SUBMENU.set(channels_submenu.clone()).ok();

    let menu = MenuBuilder::new(app)
        .item(&open_item)
        .separator()
        .item(&downloads_item)
        .item(&channels_submenu)
        .separator()
        .item(&quit_item)
        .build()?;

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("app icon not found");

    BASE_ICON
        .set((icon.rgba().to_vec(), icon.width(), icon.height()))
        .ok();

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("OmniGet")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_window(app),
            "quit" => {
                request_quit(app);
            }
            other => {
                if let Some(channel_id) = other.strip_prefix("chk:") {
                    let app = app.clone();
                    let channel_id = channel_id.to_string();
                    tauri::async_runtime::spawn(async move {
                        let _ = crate::core::channel_poller::check_now(&app, &channel_id).await;
                    });
                }
            }
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

// Repopulates the Channels submenu with the localized labels the frontend
// resolved via $t. Each entry id is "chk:{channel_id}" so on_menu_event can
// route a click to an immediate check. Only the submenu is touched — the rest
// of the tray menu (and DOWNLOADS_ITEM) is left intact.
pub fn rebuild_menu(
    app: &AppHandle,
    header: String,
    channels: Vec<(String, String)>,
) -> tauri::Result<()> {
    let Some(submenu) = CHANNELS_SUBMENU.get() else {
        return Ok(());
    };
    submenu.set_text(header)?;
    while !submenu.items()?.is_empty() {
        submenu.remove_at(0)?;
    }
    for (id, title) in &channels {
        let item = MenuItemBuilder::with_id(format!("chk:{}", id), title).build(app)?;
        submenu.append(&item)?;
    }
    submenu.set_enabled(!channels.is_empty()).ok();
    Ok(())
}

pub fn update_active_count(app: &AppHandle, count: u32) {
    if let Some(item) = DOWNLOADS_ITEM.get() {
        let _ = item.set_text(active_label(ui_lang(app), count));
    }

    let prev = ICON_COUNT.swap(count, Ordering::Relaxed);
    if prev == count {
        return;
    }
    if count == 0 {
        LAST_SPEED_BUCKET.store(u64::MAX, Ordering::Relaxed);
    }

    if let Some(tray) = app.tray_by_id("main-tray") {
        let lang = ui_lang(app);
        let tooltip = if count > 0 {
            if lang == "ru" {
                format!("OmniGet — активных: {}", count)
            } else {
                format!("OmniGet — {} active", count)
            }
        } else {
            "OmniGet".into()
        };
        let _ = tray.set_tooltip(Some(&tooltip));

        if let Some((base, w, h)) = BASE_ICON.get() {
            let rgba = if count > 0 {
                let cache = BADGE_CACHE.get_or_init(|| Mutex::new(BadgeCache::new()));
                if let Ok(mut c) = cache.lock() {
                    c.get_or_render(base, *w, *h, count)
                } else {
                    render_badge(base, *w, *h, count)
                }
            } else {
                base.clone()
            };
            let _ = tray.set_icon(Some(Image::new_owned(rgba, *w, *h)));
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn format_speed(bps: f64) -> String {
    if bps < 1024.0 * 1024.0 {
        format!("{:.0} KB/s", bps / 1024.0)
    } else {
        format!("{:.1} MB/s", bps / (1024.0 * 1024.0))
    }
}

// Speed lives only in the tooltip (the discreet channel) — the icon stays
// count-gated because icon re-render is expensive. Writes are hard-throttled
// and deduped so frequent progress ticks can't flood the system tray.
pub fn update_speed_tooltip(app: &AppHandle, count: u32, total_speed_bps: f64) {
    if count == 0 {
        return;
    }

    let now = now_ms();
    let last = LAST_TOOLTIP_MS.load(Ordering::Relaxed);
    if now.saturating_sub(last) < SPEED_TOOLTIP_MIN_INTERVAL_MS {
        return;
    }

    let speed_tenths = (total_speed_bps / (1024.0 * 1024.0) * 10.0).round() as u64;
    let bucket = (count as u64) << 32 | speed_tenths;
    if LAST_SPEED_BUCKET.load(Ordering::Relaxed) == bucket {
        return;
    }
    LAST_SPEED_BUCKET.store(bucket, Ordering::Relaxed);
    LAST_TOOLTIP_MS.store(now, Ordering::Relaxed);

    if let Some(tray) = app.tray_by_id("main-tray") {
        let lang = ui_lang(app);
        let tooltip = if total_speed_bps > 0.0 {
            if lang == "ru" {
                format!(
                    "OmniGet — активных: {} · {}",
                    count,
                    format_speed(total_speed_bps)
                )
            } else {
                format!(
                    "OmniGet — {} active · {}",
                    count,
                    format_speed(total_speed_bps)
                )
            }
        } else if lang == "ru" {
            format!("OmniGet — активных: {}", count)
        } else {
            format!("OmniGet — {} active", count)
        };
        let _ = tray.set_tooltip(Some(&tooltip));
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

fn active_label(lang: &str, count: u32) -> String {
    if count == 0 {
        tr(lang, "No active downloads", "Нет активных загрузок").into()
    } else if lang == "ru" {
        format!("Активных загрузок: {}", count)
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

pub fn request_quit(app: &AppHandle) {
    let active = compute_total_active(app);
    if active == 0 {
        app.exit(0);
        return;
    }
    show_window(app);
    let _ = app.emit("exit-confirm-required", active);
}

fn glyph(ch: char) -> [[bool; 3]; 5] {
    match ch {
        '0' => [
            [true, true, true],
            [true, false, true],
            [true, false, true],
            [true, false, true],
            [true, true, true],
        ],
        '1' => [
            [false, true, false],
            [true, true, false],
            [false, true, false],
            [false, true, false],
            [true, true, true],
        ],
        '2' => [
            [true, true, true],
            [false, false, true],
            [true, true, true],
            [true, false, false],
            [true, true, true],
        ],
        '3' => [
            [true, true, true],
            [false, false, true],
            [true, true, true],
            [false, false, true],
            [true, true, true],
        ],
        '4' => [
            [true, false, true],
            [true, false, true],
            [true, true, true],
            [false, false, true],
            [false, false, true],
        ],
        '5' => [
            [true, true, true],
            [true, false, false],
            [true, true, true],
            [false, false, true],
            [true, true, true],
        ],
        '6' => [
            [true, true, true],
            [true, false, false],
            [true, true, true],
            [true, false, true],
            [true, true, true],
        ],
        '7' => [
            [true, true, true],
            [false, false, true],
            [false, true, false],
            [false, true, false],
            [false, true, false],
        ],
        '8' => [
            [true, true, true],
            [true, false, true],
            [true, true, true],
            [true, false, true],
            [true, true, true],
        ],
        '9' => [
            [true, true, true],
            [true, false, true],
            [true, true, true],
            [false, false, true],
            [true, true, true],
        ],
        '+' => [
            [false, false, false],
            [false, true, false],
            [true, true, true],
            [false, true, false],
            [false, false, false],
        ],
        _ => [[false; 3]; 5],
    }
}

const GLYPH_W: u32 = 3;
const GLYPH_H: u32 = 5;

fn render_badge(base: &[u8], w: u32, h: u32, count: u32) -> Vec<u8> {
    let mut buf = base.to_vec();
    let size = w.min(h) as f32;

    let radius = (size * 0.35).max(7.0);
    let cx = w as f32 - radius - 1.0;
    let cy = h as f32 - radius - 1.0;

    let (br, bg, bb) = (237u8, 34, 54);

    let imin = (cy - radius - 1.0).max(0.0) as u32;
    let imax = ((cy + radius + 1.0) as u32).min(h - 1);
    let jmin = (cx - radius - 1.0).max(0.0) as u32;
    let jmax = ((cx + radius + 1.0) as u32).min(w - 1);

    for y in imin..=imax {
        for x in jmin..=jmax {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d2 = dx * dx + dy * dy;
            if d2 <= (radius + 1.0) * (radius + 1.0) {
                let idx = ((y * w + x) * 4) as usize;
                if idx + 3 >= buf.len() {
                    continue;
                }
                let dist = d2.sqrt();
                let edge = radius - dist;
                let alpha = if edge >= 1.0 {
                    255.0
                } else if edge > 0.0 {
                    edge * 255.0
                } else {
                    continue;
                };
                let a = alpha / 255.0;
                buf[idx] = (br as f32 * a + buf[idx] as f32 * (1.0 - a)) as u8;
                buf[idx + 1] = (bg as f32 * a + buf[idx + 1] as f32 * (1.0 - a)) as u8;
                buf[idx + 2] = (bb as f32 * a + buf[idx + 2] as f32 * (1.0 - a)) as u8;
                buf[idx + 3] = buf[idx + 3].max(alpha as u8);
            }
        }
    }

    let chars: Vec<char> = if count > 9 {
        vec!['9', '+']
    } else {
        vec![char::from_digit(count, 10).unwrap_or('0')]
    };

    let scale = ((size / 11.0).round() as u32).max(1);

    let gap = if chars.len() > 1 { scale } else { 0 };
    let text_w = chars.len() as u32 * GLYPH_W * scale + gap;
    let text_h = GLYPH_H * scale;

    let text_x = cx as i32 - text_w as i32 / 2;
    let text_y = cy as i32 - text_h as i32 / 2;

    let mut ox = text_x;
    for ch in &chars {
        let g = glyph(*ch);
        for row in 0..GLYPH_H {
            for col in 0..GLYPH_W {
                if g[row as usize][col as usize] {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let px = ox + (col * scale + sx) as i32;
                            let py = text_y + (row * scale + sy) as i32;
                            if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                                let idx = ((py as u32 * w + px as u32) * 4) as usize;
                                if idx + 3 < buf.len() {
                                    buf[idx] = 255;
                                    buf[idx + 1] = 255;
                                    buf[idx + 2] = 255;
                                    buf[idx + 3] = 255;
                                }
                            }
                        }
                    }
                }
            }
        }
        ox += (GLYPH_W * scale) as i32 + gap as i32;
    }

    buf
}

#[cfg(target_os = "windows")]
fn render_overlay_badge(_base: &[u8], w: u32, h: u32, count: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (w * h * 4) as usize];

    let size = w.min(h) as f32;
    let radius = (size * 0.45).max(3.5);
    let cx = (w as f32) * 0.75;
    let cy = (h as f32) * 0.75;

    let (br, bg, bb) = (237u8, 34, 54);

    let imin = (cy - radius - 1.0).max(0.0) as u32;
    let imax = ((cy + radius + 1.0) as u32).min(h - 1);
    let jmin = (cx - radius - 1.0).max(0.0) as u32;
    let jmax = ((cx + radius + 1.0) as u32).min(w - 1);

    for y in imin..=imax {
        for x in jmin..=jmax {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d2 = dx * dx + dy * dy;
            if d2 <= (radius + 1.0) * (radius + 1.0) {
                let idx = ((y * w + x) * 4) as usize;
                if idx + 3 >= buf.len() {
                    continue;
                }
                let dist = d2.sqrt();
                let edge = radius - dist;
                let alpha = if edge >= 1.0 {
                    255.0
                } else if edge > 0.0 {
                    edge * 255.0
                } else {
                    continue;
                };
                let a = alpha / 255.0;
                buf[idx] = (br as f32 * a) as u8;
                buf[idx + 1] = (bg as f32 * a) as u8;
                buf[idx + 2] = (bb as f32 * a) as u8;
                buf[idx + 3] = (alpha as u8).max(200);
            }
        }
    }

    let chars: Vec<char> = if count > 9 {
        vec!['9', '+']
    } else {
        vec![char::from_digit(count, 10).unwrap_or('0')]
    };

    let scale = ((size / 8.0).round() as u32).max(1);
    let gap = if chars.len() > 1 { scale } else { 0 };
    let text_w = chars.len() as u32 * GLYPH_W * scale + gap;
    let text_h = GLYPH_H * scale;

    let text_x = cx as i32 - text_w as i32 / 2;
    let text_y = cy as i32 - text_h as i32 / 2;

    let mut ox = text_x;
    for ch in &chars {
        let g = glyph(*ch);
        for row in 0..GLYPH_H {
            for col in 0..GLYPH_W {
                if g[row as usize][col as usize] {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let px = ox + (col * scale + sx) as i32;
                            let py = text_y + (row * scale + sy) as i32;
                            if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                                let idx = ((py as u32 * w + px as u32) * 4) as usize;
                                if idx + 3 < buf.len() {
                                    buf[idx] = 255;
                                    buf[idx + 1] = 255;
                                    buf[idx + 2] = 255;
                                    buf[idx + 3] = 255;
                                }
                            }
                        }
                    }
                }
            }
        }
        ox += (GLYPH_W * scale) as i32 + gap as i32;
    }

    buf
}

pub fn update_taskbar_badge(app: &AppHandle, active_count: u32, _avg_percent: f64) {
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "macos")]
        {
            let badge_count = if active_count > 0 {
                Some(active_count as i64)
            } else {
                None
            };
            let _ = window.set_badge_count(badge_count);
        }

        #[cfg(target_os = "windows")]
        {
            use tauri::window::ProgressBarState;
            if active_count > 0 {
                if let Some((base, _, _)) = BASE_ICON.get() {
                    let overlay = render_overlay_badge(base, 16, 16, active_count);
                    let _ = window.set_overlay_icon(Some(Image::new_owned(overlay, 16, 16)));
                }
                let progress_val = (_avg_percent.clamp(0.0, 1.0) * 100.0) as u64;
                let _ = window.set_progress_bar(ProgressBarState {
                    progress: Some(progress_val),
                    status: None,
                });
            } else {
                let _ = window.set_overlay_icon(None);
                let _ = window.set_progress_bar(ProgressBarState {
                    progress: None,
                    status: None,
                });
            }
        }

        #[cfg(target_os = "linux")]
        {
            let badge_count = if active_count > 0 {
                Some(active_count as i64)
            } else {
                None
            };
            let _ = window.set_badge_count(badge_count);
        }
    }
}
