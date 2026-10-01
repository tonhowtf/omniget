use serde::Serialize;

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MacGameMode {
    pub on: Option<bool>,
    pub policy: Option<String>,
}

pub fn gamepolicyctl_status(text: &str) -> MacGameMode {
    let clean = strip_ansi(text);
    let mut on = None;
    let mut policy = None;
    for line in clean.lines() {
        let l = line.trim();
        let lower = l.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("game mode is ") {
            let word = rest
                .trim_end_matches('.')
                .split_whitespace()
                .next()
                .unwrap_or("");
            on = match word {
                "on" => Some(true),
                "off" => Some(false),
                _ => None,
            };
        } else if let Some(i) = lower.find("policy is currently ") {
            let rest = &lower[i + "policy is currently ".len()..];
            let word = rest
                .split(|c: char| c == '.' || c.is_whitespace())
                .next()
                .unwrap_or("");
            if matches!(word, "automatic" | "on" | "off") {
                policy = Some(word.to_string());
            }
        }
    }
    MacGameMode { on, policy }
}

pub fn auto_game_mode(reg: &str) -> Option<bool> {
    super::super::parse::reg_query(reg)
        .into_iter()
        .find(|v| v.name.eq_ignore_ascii_case("AutoGameModeEnabled"))
        .and_then(|v| {
            let d = v.data.trim();
            let n = d
                .strip_prefix("0x")
                .map(|h| u32::from_str_radix(h, 16).ok())
                .unwrap_or_else(|| d.parse().ok())?;
            Some(n != 0)
        })
}

pub fn install_command(managers: &[String]) -> Option<(String, Vec<String>)> {
    let pick = |m: &str| -> Option<Vec<&'static str>> {
        Some(match m {
            "apt" => vec!["sudo", "apt", "install", "gamemode"],
            "dnf" => vec!["sudo", "dnf", "install", "gamemode"],
            "pacman" => vec!["sudo", "pacman", "-S", "gamemode"],
            "zypper" => vec!["sudo", "zypper", "install", "gamemode"],
            _ => return None,
        })
    };
    managers
        .iter()
        .find_map(|m| pick(m).map(|argv| (m.clone(), argv.into_iter().map(String::from).collect())))
}

pub fn guid_string(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> String {
    format!(
        "{d1:08x}-{d2:04x}-{d3:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]
    )
}

pub fn parse_guid(s: &str) -> Option<(u32, u16, u16, [u8; 8])> {
    let s = s.trim().trim_start_matches('{').trim_end_matches('}');
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5
        || parts[0].len() != 8
        || parts[1].len() != 4
        || parts[2].len() != 4
        || parts[3].len() != 4
        || parts[4].len() != 12
    {
        return None;
    }
    let d1 = u32::from_str_radix(parts[0], 16).ok()?;
    let d2 = u16::from_str_radix(parts[1], 16).ok()?;
    let d3 = u16::from_str_radix(parts[2], 16).ok()?;
    let tail = format!("{}{}", parts[3], parts[4]);
    let mut d4 = [0u8; 8];
    for (i, b) in d4.iter_mut().enumerate() {
        *b = u8::from_str_radix(tail.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some((d1, d2, d3, d4))
}

pub const SCHEME_HIGH_PERFORMANCE: &str = "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c";
pub const SCHEME_ULTIMATE: &str = "e9a42b02-d5df-448d-aa00-03f14749eb61";

#[cfg(test)]
mod tests {
    use super::*;

    const OFF: &str =
        include_str!("../../../../tests/washmybelly_fixtures/game/gamepolicyctl_status_off.txt");
    const FORCED: &str = include_str!(
        "../../../../tests/washmybelly_fixtures/game/gamepolicyctl_status_forced_on.txt"
    );
    const REG_OFF: &str =
        include_str!("../../../../tests/washmybelly_fixtures/game/reg_gamebar_off.txt");
    const REG_ON: &str =
        include_str!("../../../../tests/washmybelly_fixtures/game/reg_gamebar_on.txt");

    #[test]
    fn mac_status_real_output_is_off_and_automatic() {
        let s = gamepolicyctl_status(OFF);
        assert_eq!(s.on, Some(false));
        assert_eq!(s.policy.as_deref(), Some("automatic"));
    }

    #[test]
    fn mac_status_forced_on() {
        let s = gamepolicyctl_status(FORCED);
        assert_eq!(s.on, Some(true));
        assert_eq!(s.policy.as_deref(), Some("on"));
    }

    #[test]
    fn mac_status_unknown_text_says_unknown() {
        let s = gamepolicyctl_status("something else\n");
        assert_eq!(
            s,
            MacGameMode {
                on: None,
                policy: None
            }
        );
    }

    #[test]
    fn ansi_is_stripped() {
        assert_eq!(strip_ansi("a\u{1b}[0;31mred\u{1b}[0;0m."), "ared.");
    }

    #[test]
    fn windows_auto_game_mode_values() {
        assert_eq!(auto_game_mode(REG_OFF), Some(false));
        assert_eq!(auto_game_mode(REG_ON), Some(true));
        assert_eq!(auto_game_mode(""), None);
    }

    #[test]
    fn install_hint_follows_the_package_manager() {
        let (m, argv) = install_command(&["flatpak".into(), "dnf".into()]).unwrap();
        assert_eq!(m, "dnf");
        assert_eq!(argv, ["sudo", "dnf", "install", "gamemode"]);
        assert_eq!(install_command(&["pacman".into()]).unwrap().1[2], "-S");
        assert!(install_command(&["snap".into()]).is_none());
        assert!(install_command(&[]).is_none());
    }

    #[test]
    fn guid_round_trip() {
        let g = parse_guid("{8C5E7FDA-E8BF-4A96-9A85-A6E23A8C635C}").unwrap();
        assert_eq!(g.0, 0x8c5e7fda);
        assert_eq!(g.3, [0x9a, 0x85, 0xa6, 0xe2, 0x3a, 0x8c, 0x63, 0x5c]);
        assert_eq!(guid_string(g.0, g.1, g.2, g.3), SCHEME_HIGH_PERFORMANCE);
        assert!(parse_guid("not-a-guid").is_none());
        assert!(parse_guid("8c5e7fda-e8bf-4a96-9a85-a6e23a8c635").is_none());
    }
}
