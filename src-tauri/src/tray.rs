use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItem, MenuItemBuilder},
    tray::TrayIconBuilder,
    window::ProgressBarState,
    AppHandle, Manager, Wry,
};

static DOWNLOADS_ITEM: OnceLock<MenuItem<Wry>> = OnceLock::new();
static BASE_ICON: OnceLock<(Vec<u8>, u32, u32)> = OnceLock::new();
static LAST_ACTIVE: AtomicU32 = AtomicU32::new(0);
static ICON_COUNT: AtomicU32 = AtomicU32::new(0);

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

pub fn update_active_count(app: &AppHandle, count: u32) {
    if let Some(item) = DOWNLOADS_ITEM.get() {
        let _ = item.set_text(active_label(count));
    }

    let prev = ICON_COUNT.swap(count, Ordering::Relaxed);
    if prev == count {
        return;
    }

    if let Some(tray) = app.tray_by_id("main-tray") {
        let tooltip = if count > 0 {
            format!("OmniGet — {} active", count)
        } else {
            "OmniGet".into()
        };
        let _ = tray.set_tooltip(Some(&tooltip));

        if let Some((base, w, h)) = BASE_ICON.get() {
            let rgba = if count > 0 {
                render_badge(base, *w, *h, count)
            } else {
                base.clone()
            };
            let _ = tray.set_icon(Some(Image::new_owned(rgba, *w, *h)));
        }
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

    let radius = (size * 0.22).max(5.0);
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

    let scale = ((size / 16.0).round() as u32).max(1);

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
                            if px >= 0
                                && py >= 0
                                && (px as u32) < w
                                && (py as u32) < h
                            {
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

fn render_overlay_badge(_base: &[u8], w: u32, h: u32, count: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (w * h * 4) as usize];

    let size = w.min(h) as f32;
    let radius = (size * 0.35).max(2.0);
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

    let scale = ((size / 12.0).round() as u32).max(1);
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
                            if px >= 0
                                && py >= 0
                                && (px as u32) < w
                                && (py as u32) < h
                            {
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

pub fn update_taskbar_badge(app: &AppHandle, active_count: u32, avg_percent: f64) {
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "macos")]
        {
            let badge_count = if active_count > 0 {
                Some(active_count as usize)
            } else {
                None
            };
            let _ = window.set_badge_count(badge_count);
        }

        #[cfg(target_os = "windows")]
        {
            if active_count > 0 {
                if let Some((base, _, _)) = BASE_ICON.get() {
                    let overlay = render_overlay_badge(base, 16, 16, active_count);
                    let _ = window.set_overlay_icon(Some(Image::new_owned(overlay, 16, 16)));
                }
                let progress_val = (avg_percent.clamp(0.0, 1.0) * 100.0) as u64;
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
                Some(active_count as usize)
            } else {
                None
            };
            let _ = window.set_badge_count(badge_count);
        }
    }
}
