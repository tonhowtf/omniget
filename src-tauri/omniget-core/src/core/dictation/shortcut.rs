#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutError {
    Empty,
    NoKey,
    TooManyKeys,
    UnknownKey,
    NeedsModifier,
    Reserved,
}

impl ShortcutError {
    pub fn code(self) -> &'static str {
        match self {
            ShortcutError::Empty => "EMPTY",
            ShortcutError::NoKey => "NO_KEY",
            ShortcutError::TooManyKeys => "TOO_MANY_KEYS",
            ShortcutError::UnknownKey => "UNKNOWN_KEY",
            ShortcutError::NeedsModifier => "NEEDS_MODIFIER",
            ShortcutError::Reserved => "RESERVED",
        }
    }
}

pub fn default_shortcut(mac: bool) -> &'static str {
    if mac {
        "Alt+Space"
    } else {
        "Ctrl+Alt+Space"
    }
}

const MODIFIERS: [&str; 4] = ["Ctrl", "Alt", "Shift", "Super"];

fn modifier(token: &str, mac: bool) -> Option<&'static str> {
    match token {
        "CTRL" | "CONTROL" => Some("Ctrl"),
        "ALT" | "OPTION" | "OPT" => Some("Alt"),
        "SHIFT" => Some("Shift"),
        "SUPER" | "CMD" | "COMMAND" | "META" | "WIN" | "WINDOWS" => Some("Super"),
        "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCTRL" | "CMDORCONTROL" => {
            Some(if mac { "Super" } else { "Ctrl" })
        }
        _ => None,
    }
}

fn key(token: &str) -> Option<String> {
    let named = match token {
        "SPACE" | " " => "Space",
        "ENTER" | "RETURN" => "Enter",
        "TAB" => "Tab",
        "BACKSPACE" => "Backspace",
        "DELETE" | "DEL" => "Delete",
        "HOME" => "Home",
        "END" => "End",
        "PAGEUP" => "PageUp",
        "PAGEDOWN" => "PageDown",
        "INSERT" => "Insert",
        "UP" | "ARROWUP" => "Up",
        "DOWN" | "ARROWDOWN" => "Down",
        "LEFT" | "ARROWLEFT" => "Left",
        "RIGHT" | "ARROWRIGHT" => "Right",
        "COMMA" | "," => "Comma",
        "PERIOD" | "." => "Period",
        "SLASH" | "/" => "Slash",
        "BACKSLASH" | "\\" => "Backslash",
        "SEMICOLON" | ";" => "Semicolon",
        "QUOTE" | "'" => "Quote",
        "BRACKETLEFT" | "[" => "BracketLeft",
        "BRACKETRIGHT" | "]" => "BracketRight",
        "BACKQUOTE" | "`" => "Backquote",
        "MINUS" | "-" => "Minus",
        "EQUAL" | "=" => "Equal",
        "ESC" | "ESCAPE" => "Escape",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.to_string());
    }
    let bare = token
        .strip_prefix("KEY")
        .filter(|s| s.len() == 1)
        .or_else(|| token.strip_prefix("DIGIT").filter(|s| s.len() == 1))
        .unwrap_or(token);
    if bare.len() == 1 {
        let c = bare.chars().next()?;
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_uppercase().to_string());
        }
        return None;
    }
    let n = token.strip_prefix('F')?.parse::<u8>().ok()?;
    (1..=24).contains(&n).then(|| format!("F{n}"))
}

fn function_number(key: &str) -> Option<u8> {
    key.strip_prefix('F')?.parse().ok()
}

fn reserved(mods: &[&str], key: &str, mac: bool) -> bool {
    if key == "Escape" {
        return true;
    }
    let only = |m: &str| mods.len() == 1 && mods[0] == m;
    let combo = |m: &str, keys: &[&str]| only(m) && keys.contains(&key);
    if mac {
        combo(
            "Super",
            &["Q", "W", "Tab", "Space", "C", "V", "X", "Z", "A", "H", "M"],
        ) || (mods == ["Ctrl", "Super"] && key == "Q")
    } else {
        combo("Alt", &["Tab", "F4"])
            || combo("Ctrl", &["C", "V", "X", "Z", "A", "Y"])
            || combo("Super", &["L", "D", "E", "R", "Tab"])
    }
}

pub fn normalize(input: &str, mac: bool) -> Result<String, ShortcutError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ShortcutError::Empty);
    }
    let tokens: Vec<String> = if trimmed == "+" {
        vec!["+".into()]
    } else {
        trimmed
            .split('+')
            .map(|t| t.trim().to_uppercase())
            .filter(|t| !t.is_empty())
            .collect()
    };
    let mut mods: Vec<&'static str> = Vec::new();
    let mut main: Option<String> = None;
    for token in &tokens {
        if let Some(m) = modifier(token, mac) {
            if !mods.contains(&m) {
                mods.push(m);
            }
            continue;
        }
        let parsed = key(token).ok_or(ShortcutError::UnknownKey)?;
        if main.is_some() {
            return Err(ShortcutError::TooManyKeys);
        }
        main = Some(parsed);
    }
    let main = main.ok_or(ShortcutError::NoKey)?;
    mods.sort_by_key(|m| MODIFIERS.iter().position(|o| o == m));
    if reserved(&mods, &main, mac) {
        return Err(ShortcutError::Reserved);
    }
    let real_modifier = mods.iter().any(|m| *m != "Shift");
    let lone_function = function_number(&main).is_some_and(|n| n >= 13);
    if !real_modifier && !lone_function {
        return Err(ShortcutError::NeedsModifier);
    }
    let mut parts: Vec<String> = mods.iter().map(|m| m.to_string()).collect();
    parts.push(main);
    Ok(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_aliases_and_order() {
        assert_eq!(normalize("alt+space", true).unwrap(), "Alt+Space");
        assert_eq!(normalize(" option + SPACE ", true).unwrap(), "Alt+Space");
        assert_eq!(normalize("shift+ctrl+keyd", false).unwrap(), "Ctrl+Shift+D");
        assert_eq!(normalize("Cmd+Alt+digit1", true).unwrap(), "Alt+Super+1");
        assert_eq!(normalize("ctrl+alt+d", false).unwrap(), "Ctrl+Alt+D");
        assert_eq!(normalize("Ctrl+ArrowUp", false).unwrap(), "Ctrl+Up");
        assert_eq!(normalize("Alt+,", false).unwrap(), "Alt+Comma");
    }

    #[test]
    fn command_or_control_depends_on_platform() {
        assert_eq!(
            normalize("CommandOrControl+Shift+K", true).unwrap(),
            "Shift+Super+K"
        );
        assert_eq!(
            normalize("CommandOrControl+Shift+K", false).unwrap(),
            "Ctrl+Shift+K"
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(normalize("   ", true), Err(ShortcutError::Empty));
        assert_eq!(normalize("Ctrl+Alt", true), Err(ShortcutError::NoKey));
        assert_eq!(
            normalize("Ctrl+A+B", false),
            Err(ShortcutError::TooManyKeys)
        );
        assert_eq!(
            normalize("Ctrl+Banana", false),
            Err(ShortcutError::UnknownKey)
        );
        assert_eq!(normalize("Ctrl+F25", false), Err(ShortcutError::UnknownKey));
        assert_eq!(normalize("D", false), Err(ShortcutError::NeedsModifier));
        assert_eq!(
            normalize("Shift+D", false),
            Err(ShortcutError::NeedsModifier)
        );
        assert_eq!(normalize("F5", false), Err(ShortcutError::NeedsModifier));
    }

    #[test]
    fn lone_high_function_keys_are_fine() {
        assert_eq!(normalize("f13", true).unwrap(), "F13");
        assert_eq!(normalize("Shift+F20", false).unwrap(), "Shift+F20");
    }

    #[test]
    fn rejects_system_shortcuts() {
        assert_eq!(normalize("Cmd+Space", true), Err(ShortcutError::Reserved));
        assert_eq!(normalize("Cmd+Q", true), Err(ShortcutError::Reserved));
        assert_eq!(normalize("Ctrl+V", false), Err(ShortcutError::Reserved));
        assert_eq!(normalize("Alt+Tab", false), Err(ShortcutError::Reserved));
        assert_eq!(
            normalize("Ctrl+Escape", false),
            Err(ShortcutError::Reserved)
        );
        assert!(normalize("Ctrl+V", true).is_ok());
    }

    #[test]
    fn defaults_are_valid() {
        assert_eq!(
            normalize(default_shortcut(true), true).unwrap(),
            "Alt+Space"
        );
        assert_eq!(
            normalize(default_shortcut(false), false).unwrap(),
            "Ctrl+Alt+Space"
        );
    }
}
