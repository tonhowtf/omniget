//! Keys, text, paste, mouse and wheel → the bytes the program expects, by the
//! terminal's current modes. The front sends DOM-level events; encoding here
//! means no race between a mode in JS and the mode the program just set.

use serde::Deserialize;

use super::engine::Modes;

pub const MOD_SHIFT: u8 = 1;
pub const MOD_ALT: u8 = 2;
pub const MOD_CTRL: u8 = 4;
pub const MOD_META: u8 = 8;

/// A non-text key from `keydown` (text goes through `term_text`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct KeyInput {
    /// DOM `KeyboardEvent.key` ("Enter", "ArrowUp", "a", "F5").
    pub key: String,
    /// DOM `KeyboardEvent.code` ("KeyA"), used for Ctrl/Alt + letter when
    /// the layout gives an odd `key`.
    pub code: String,
    pub mods: u8,
    pub repeat: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MouseInput {
    /// "down" | "up" | "move" | "wheel".
    pub kind: String,
    /// 0 left, 1 middle, 2 right.
    pub button: u8,
    pub col: u16,
    pub row: u16,
    pub mods: u8,
    /// Wheel lines: positive = up.
    pub wheel: i32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ScrollInput {
    /// Positive = towards history (up).
    pub lines: i32,
    pub pages: i32,
    /// "top" | "bottom".
    pub to: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SelectInput {
    /// "start" | "update" | "clear" | "all".
    pub kind: String,
    /// "simple" | "block" | "semantic" | "lines".
    pub ty: Option<String>,
    pub col: u16,
    pub row: u16,
    /// "left" | "right": which half of the cell the pointer is on.
    pub side: Option<String>,
}

/// xterm modifier parameter (1 + shift + 2·alt + 4·ctrl + 8·meta).
fn mod_param(mods: u8) -> u8 {
    1 + (mods & MOD_SHIFT != 0) as u8
        + 2 * (mods & MOD_ALT != 0) as u8
        + 4 * (mods & MOD_CTRL != 0) as u8
        + 8 * (mods & MOD_META != 0) as u8
}

/// `CSI 1;m X` or, without modifiers, `CSI X` / `SS3 X` in app cursor mode.
fn cursor_key(fin: u8, mods: u8, app: bool) -> Vec<u8> {
    if mods == 0 {
        if app {
            vec![0x1b, b'O', fin]
        } else {
            vec![0x1b, b'[', fin]
        }
    } else {
        format!("\x1b[1;{}{}", mod_param(mods), fin as char).into_bytes()
    }
}

/// `CSI n ~` with optional modifiers.
fn tilde_key(n: u8, mods: u8) -> Vec<u8> {
    if mods == 0 {
        format!("\x1b[{n}~").into_bytes()
    } else {
        format!("\x1b[{n};{}~", mod_param(mods)).into_bytes()
    }
}

fn with_alt(mods: u8, bytes: Vec<u8>) -> Vec<u8> {
    if mods & MOD_ALT != 0 {
        let mut v = vec![0x1b];
        v.extend(bytes);
        v
    } else {
        bytes
    }
}

/// Ctrl + a printable character → C0 byte.
fn ctrl_byte(c: char) -> Option<u8> {
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        'A'..='Z' => Some(c as u8 - b'A' + 1),
        '@' | ' ' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

/// Encodes a key. `None` = nothing to send (a bare modifier, or a key the
/// terminal does not map).
pub fn encode_key(k: &KeyInput, modes: &Modes) -> Option<Vec<u8>> {
    let mods = k.mods;
    let app = modes.app_cursor;
    let key = k.key.as_str();
    let bytes = match key {
        "Enter" => with_alt(mods, vec![b'\r']),
        "Tab" => {
            if mods & MOD_SHIFT != 0 {
                b"\x1b[Z".to_vec()
            } else {
                with_alt(mods, vec![b'\t'])
            }
        }
        "Backspace" => {
            if mods & MOD_CTRL != 0 {
                with_alt(mods, vec![0x08])
            } else {
                with_alt(mods, vec![0x7f])
            }
        }
        "Escape" => with_alt(mods, vec![0x1b]),
        "ArrowUp" => cursor_key(b'A', mods, app),
        "ArrowDown" => cursor_key(b'B', mods, app),
        "ArrowRight" => cursor_key(b'C', mods, app),
        "ArrowLeft" => cursor_key(b'D', mods, app),
        "Home" => cursor_key(b'H', mods, app),
        "End" => cursor_key(b'F', mods, app),
        "Insert" => tilde_key(2, mods),
        "Delete" => tilde_key(3, mods),
        "PageUp" => tilde_key(5, mods),
        "PageDown" => tilde_key(6, mods),
        "F1" | "F2" | "F3" | "F4" => {
            let fin = b'P' + (key.as_bytes()[1] - b'1');
            if mods == 0 {
                vec![0x1b, b'O', fin]
            } else {
                format!("\x1b[1;{}{}", mod_param(mods), fin as char).into_bytes()
            }
        }
        _ if key.len() >= 2 && key.starts_with('F') && key[1..].parse::<u8>().is_ok() => {
            let n: u8 = key[1..].parse().ok()?;
            let code = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return None,
            };
            tilde_key(code, mods)
        }
        _ => {
            let mut chars = key.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                // "Shift", "Meta", "Dead", "Unidentified"…
                return None;
            }
            // Layout-independent letter for Ctrl/Alt chords.
            let base = if mods & (MOD_CTRL | MOD_ALT) != 0 {
                k.code
                    .strip_prefix("Key")
                    .and_then(|l| l.chars().next())
                    .filter(|l| l.is_ascii_alphabetic())
                    .map(|l| {
                        if mods & MOD_SHIFT != 0 {
                            l.to_ascii_uppercase()
                        } else {
                            l.to_ascii_lowercase()
                        }
                    })
                    .unwrap_or(c)
            } else {
                c
            };
            if mods & MOD_CTRL != 0 {
                let b = ctrl_byte(base)?;
                with_alt(mods, vec![b])
            } else if mods & MOD_ALT != 0 {
                let mut v = vec![0x1b];
                let mut buf = [0u8; 4];
                v.extend_from_slice(base.encode_utf8(&mut buf).as_bytes());
                v
            } else {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf).as_bytes().to_vec()
            }
        }
    };
    Some(bytes)
}

/// Removes what could escape a bracketed paste (ESC and C1 CSI) and
/// normalises newlines to CR, then wraps in `ESC[200~ … ESC[201~` when the
/// program asked for bracketed paste.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let clean: String = text
        .replace("\r\n", "\r")
        .replace('\n', "\r")
        .chars()
        .filter(|&c| c != '\x1b' && c != '\u{9b}')
        .collect();
    if bracketed {
        let mut v = b"\x1b[200~".to_vec();
        v.extend_from_slice(clean.as_bytes());
        v.extend_from_slice(b"\x1b[201~");
        v
    } else {
        clean.into_bytes()
    }
}

/// Mouse event → report bytes, if the program asked for this kind of event.
pub fn encode_mouse(m: &MouseInput, modes: &Modes) -> Option<Vec<u8>> {
    if !modes.mouse_reporting() {
        return None;
    }
    let mut mods_bits = 0u8;
    if m.mods & MOD_SHIFT != 0 {
        mods_bits |= 4;
    }
    if m.mods & (MOD_ALT | MOD_META) != 0 {
        mods_bits |= 8;
    }
    if m.mods & MOD_CTRL != 0 {
        mods_bits |= 16;
    }
    let (code, release) = match m.kind.as_str() {
        "down" => (m.button.min(2), false),
        "up" => (m.button.min(2), true),
        "move" => {
            if !(modes.mouse_drag || modes.mouse_motion) {
                return None;
            }
            // Drag with the button held reports the button; plain motion 3.
            let b = if m.button <= 2 && modes.mouse_drag {
                m.button
            } else if modes.mouse_motion {
                3
            } else {
                return None;
            };
            (b + 32, false)
        }
        "wheel" => {
            if m.wheel == 0 {
                return None;
            }
            (if m.wheel > 0 { 64 } else { 65 }, false)
        }
        _ => return None,
    };
    let cb = code | mods_bits;
    let x = m.col as u32 + 1;
    let y = m.row as u32 + 1;
    let repeat = if m.kind == "wheel" {
        m.wheel.unsigned_abs().clamp(1, 10) as usize
    } else {
        1
    };
    let one = if modes.sgr_mouse {
        format!("\x1b[<{cb};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes()
    } else {
        // X10/normal: release is button 3; coordinates capped at 223.
        let b = if release { 3 | mods_bits } else { cb };
        if x > 223 || y > 223 {
            return None;
        }
        vec![0x1b, b'[', b'M', 32 + b, 32 + x as u8, 32 + y as u8]
    };
    Some(one.repeat(repeat))
}

/// Wheel in the alternate screen without mouse reporting (mode 1007): arrow
/// keys, as xterm does, so `less`/`vim` scroll.
pub fn encode_alternate_scroll(lines: i32, modes: &Modes) -> Option<Vec<u8>> {
    if !(modes.alt_screen && modes.alternate_scroll) || modes.mouse_reporting() || lines == 0 {
        return None;
    }
    let fin = if lines > 0 { b'A' } else { b'B' };
    let one = cursor_key(fin, 0, modes.app_cursor);
    Some(one.repeat(lines.unsigned_abs().min(50) as usize))
}

/// Focus in/out report (mode 1004).
pub fn encode_focus(focused: bool, modes: &Modes) -> Option<Vec<u8>> {
    modes.focus_reporting.then(|| {
        if focused {
            b"\x1b[I".to_vec()
        } else {
            b"\x1b[O".to_vec()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: &str, mods: u8) -> KeyInput {
        KeyInput {
            key: k.into(),
            code: String::new(),
            mods,
            repeat: false,
        }
    }

    #[test]
    fn arrows_follow_app_cursor_mode() {
        let mut m = Modes::default();
        assert_eq!(encode_key(&key("ArrowUp", 0), &m).unwrap(), b"\x1b[A");
        m.app_cursor = true;
        assert_eq!(encode_key(&key("ArrowUp", 0), &m).unwrap(), b"\x1bOA");
        assert_eq!(
            encode_key(&key("ArrowLeft", MOD_CTRL), &m).unwrap(),
            b"\x1b[1;5D"
        );
    }

    #[test]
    fn ctrl_letters_and_paste() {
        let m = Modes::default();
        assert_eq!(encode_key(&key("c", MOD_CTRL), &m).unwrap(), vec![3]);
        assert_eq!(encode_key(&key("Tab", MOD_SHIFT), &m).unwrap(), b"\x1b[Z");
        assert_eq!(
            encode_paste("a\x1b[31m\nb", true),
            b"\x1b[200~a[31m\rb\x1b[201~"
        );
        assert_eq!(encode_paste("x\r\ny", false), b"x\ry");
    }

    #[test]
    fn sgr_mouse() {
        let m = Modes {
            mouse_click: true,
            sgr_mouse: true,
            ..Default::default()
        };
        let down = MouseInput {
            kind: "down".into(),
            col: 4,
            row: 2,
            ..Default::default()
        };
        assert_eq!(encode_mouse(&down, &m).unwrap(), b"\x1b[<0;5;3M");
        assert!(encode_mouse(&down, &Modes::default()).is_none());
    }
}
