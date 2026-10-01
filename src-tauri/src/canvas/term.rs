//! Terminal host of the canvas: the process-wide `TermManager` (shared with
//! the Central) bound to this app's events, and the `Channel` sink that
//! carries `OGT1` frames to a webview.
//!
//! One frame subscription per webview, keyed by its label; the core runs one
//! ticker thread per subscription (wakes on damage, at most ~60 frames/s,
//! credit of two frames in flight).

use std::sync::Arc;

use omniget_core::core::term::session::{FrameSink, TermManager};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::AppHandle;

/// Sends each frame as a raw `ArrayBuffer` (no JSON, no base64).
pub struct ChannelSink(pub Channel<InvokeResponseBody>);

impl FrameSink for ChannelSink {
    fn send(&self, frame: Vec<u8>) -> bool {
        self.0.send(InvokeResponseBody::Raw(frame)).is_ok()
    }
}

/// The manager, with the app's event sink bound (`term://state`,
/// `term://exit`, and the Central's `pty://*`).
pub fn host(app: &AppHandle) -> Arc<TermManager> {
    crate::pty::manager().bind(app);
    // Sleep prevention and memory limit follow the terminals (idempotent).
    super::power::start(app);
    crate::pty::term()
}

// ------------------------------------------------------------ user themes

/// Folder of the user's terminal themes (Ghostty format), read live: a file
/// dropped there shows up in the theme list without a restart.
pub fn themes_dir() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".omniget").join("terminal").join("themes"))
}

/// One user theme, in the shape of `src/lib/canvas/term/themes.ts` `TermScheme`.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct UserTheme {
    pub id: String,
    pub background: String,
    pub foreground: String,
    pub cursor: String,
    #[serde(rename = "selectionBackground")]
    pub selection_background: String,
    pub ansi: Vec<String>,
}

const DEFAULT_ANSI: [&str; 16] = [
    "#3a3a3c", "#ff453a", "#30d158", "#ffd60a", "#0a84ff", "#bf5af2", "#40c8e0", "#d1d1d6",
    "#8e8e93", "#ff6961", "#4cd964", "#ffe066", "#409cff", "#da8fff", "#70d7ff", "#ffffff",
];

fn hex(v: &str) -> Option<String> {
    let v = v.trim().trim_matches('"');
    let h = v.strip_prefix('#').unwrap_or(v);
    (h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("#{}", h.to_ascii_lowercase()))
}

/// Parses a Ghostty theme (`palette = 0=#rrggbb`, `background = …`,
/// `foreground`, `cursor-color`, `selection-background`). A theme without a
/// cursor colour uses its foreground. `None` without background/foreground.
pub fn parse_ghostty(id: &str, text: &str) -> Option<UserTheme> {
    let mut ansi: Vec<String> = DEFAULT_ANSI.iter().map(|s| s.to_string()).collect();
    let (mut bg, mut fg, mut cursor, mut sel) = (None, None, None, None);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k.trim() {
            "palette" => {
                if let Some((n, c)) = v.split_once('=') {
                    if let (Ok(n), Some(c)) = (n.trim().parse::<usize>(), hex(c)) {
                        if n < 16 {
                            ansi[n] = c;
                        }
                    }
                }
            }
            "background" => bg = hex(v),
            "foreground" => fg = hex(v),
            "cursor-color" => cursor = hex(v),
            "selection-background" => sel = hex(v),
            _ => {}
        }
    }
    let (bg, fg) = (bg?, fg?);
    Some(UserTheme {
        id: id.to_string(),
        cursor: cursor.unwrap_or_else(|| fg.clone()),
        selection_background: sel.unwrap_or_else(|| "rgba(128,128,128,0.35)".into()),
        background: bg,
        foreground: fg,
        ansi,
    })
}

/// Every valid theme in [`themes_dir`], sorted by id.
pub fn user_themes() -> Vec<UserTheme> {
    let Some(dir) = themes_dir() else {
        return Vec::new();
    };
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<UserTheme> = rd
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            let id = name.strip_suffix(".conf").unwrap_or(&name).to_string();
            let meta = e.metadata().ok()?;
            if meta.len() > 64 * 1024 {
                return None;
            }
            parse_ghostty(&id, &std::fs::read_to_string(e.path()).ok()?)
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Built-in scheme ids (mirror of `TERM_SCHEMES` in the front).
pub const BUILTIN_THEMES: [&str; 13] = [
    "system",
    "dark",
    "light",
    "dracula",
    "nord",
    "solarized-dark",
    "solarized-light",
    "gruvbox-dark",
    "tokyo-night",
    "catppuccin-mocha",
    "one-dark",
    "rose-pine",
    "everforest",
];

/// Theme ids for the settings list: built-in first, then the user's folder.
pub fn theme_ids() -> Vec<String> {
    BUILTIN_THEMES
        .iter()
        .map(|s| s.to_string())
        .chain(user_themes().into_iter().map(|t| t.id))
        .collect()
}
