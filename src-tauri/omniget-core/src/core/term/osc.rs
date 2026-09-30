//! Incremental sniffer of the sequences the emulator ignores but the canvas
//! needs: titles (OSC 0/2), cwd (OSC 7, OSC 1337 CurrentDir), notifications
//! (OSC 9, 99, 777), progress (OSC 9;4), semantic prompts (OSC 133) and BEL.
//!
//! Built on the replay tokenizer of `session::sanitize`. A sequence cut
//! between two reads is carried over (bounded) and finished by the next read,
//! so no signal is lost to the PTY's chunking.

use super::session::sanitize::{tokenize, Token};

/// Longest unfinished sequence carried between reads. Anything longer is a
/// runaway string (or a DCS/APC payload we do not care about) and is dropped.
const CARRY_MAX: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum OscEvent {
    /// OSC 0 / OSC 2.
    Title(String),
    /// OSC 7 `file://host/path` or OSC 1337 `CurrentDir=`.
    Cwd(String),
    /// OSC 9 (iTerm2), OSC 777 `notify` (rxvt, VS Code, Ghostty), OSC 99
    /// (kitty).
    Notify { title: Option<String>, body: String },
    /// OSC 9;4 (ConEmu / Windows Terminal): `state` 0 remove, 1 value,
    /// 2 error, 3 indeterminate, 4 paused/warning.
    Progress { state: u8, value: Option<u8> },
    /// OSC 133 (FinalTerm): A prompt start, B command start, C output
    /// start, D command finished (with its exit code).
    Prompt { mark: char, exit: Option<i32> },
    /// OSC 1337 `SetUserVar=name=base64`.
    UserVar { name: String, value: String },
    /// BEL outside any string.
    Bell,
}

#[derive(Default)]
pub struct Sniffer {
    carry: Vec<u8>,
}

impl Sniffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one read from the PTY and returns the events it completed.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<OscEvent> {
        let mut out = Vec::new();
        if self.carry.is_empty() && !chunk.iter().any(|&b| b == 0x1b || b == 0x07) {
            return out;
        }
        let joined;
        let data: &[u8] = if self.carry.is_empty() {
            chunk
        } else {
            let mut v = std::mem::take(&mut self.carry);
            v.extend_from_slice(chunk);
            joined = v;
            &joined
        };
        let mut rest: Option<Vec<u8>> = None;
        for tok in tokenize(data) {
            match tok {
                Token::Ctl(0x07) => out.push(OscEvent::Bell),
                Token::Osc(seq) => {
                    if let Some(ev) = parse_osc(seq) {
                        out.push(ev);
                    }
                }
                // Only OSC-like strings matter across reads; a cut CSI is
                // carried too (cheap) so the tokenizer stays aligned.
                Token::Partial(seq) if seq.len() <= CARRY_MAX => {
                    rest = Some(seq.to_vec());
                }
                _ => {}
            }
        }
        if let Some(r) = rest {
            self.carry = r;
        }
        out
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn cwd_from_url(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("file://")
        .or_else(|| url.strip_prefix("kitty-shell-cwd://"))?;
    // Skip the host part.
    let path = &rest[rest.find('/')?..];
    let mut p = percent_decode(path);
    // file:///C:/Users → /C:/Users on Windows.
    if p.len() > 3 && p.as_bytes()[0] == b'/' && p.as_bytes()[2] == b':' {
        p.remove(0);
    }
    Some(p)
}

fn b64_text(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map(|v| String::from_utf8_lossy(&v).into_owned())
        .unwrap_or_default()
}

/// Parses one complete `ESC ] … (BEL | ESC \)` sequence.
pub fn parse_osc(seq: &[u8]) -> Option<OscEvent> {
    let body = seq.strip_prefix(b"\x1b]")?;
    let body = body
        .strip_suffix(b"\x1b\\")
        .or_else(|| body.strip_suffix(&[0x07]))
        .or_else(|| body.strip_suffix(&[0x18]))
        .or_else(|| body.strip_suffix(&[0x1a]))
        .unwrap_or(body);
    let text = String::from_utf8_lossy(body);
    let (num, arg) = match text.find(';') {
        Some(i) => (&text[..i], &text[i + 1..]),
        None => (&text[..], ""),
    };
    match num {
        "0" | "2" => Some(OscEvent::Title(arg.to_string())),
        "7" => cwd_from_url(arg).map(OscEvent::Cwd),
        "9" => {
            if let Some(p) = arg
                .strip_prefix("4;")
                .or_else(|| (arg == "4").then_some(""))
            {
                let mut it = p.split(';');
                let state = it
                    .next()
                    .and_then(|s| s.trim().parse::<u8>().ok())
                    .unwrap_or(0);
                let value = it
                    .next()
                    .and_then(|s| s.trim().parse::<u16>().ok())
                    .map(|v| v.min(100) as u8);
                return Some(OscEvent::Progress { state, value });
            }
            // Other ConEmu sub-commands are numeric (`9;1;…`); real iTerm2
            // messages are text.
            let first = arg.split(';').next().unwrap_or("");
            if !first.is_empty() && first.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            Some(OscEvent::Notify {
                title: None,
                body: arg.to_string(),
            })
        }
        "99" => {
            // `99;metadata;payload`: keep the payload; `p=title` marks a title.
            let (meta, payload) = match arg.find(';') {
                Some(i) => (&arg[..i], &arg[i + 1..]),
                None => ("", arg),
            };
            if payload.is_empty() {
                return None;
            }
            let is_title = meta.split(':').any(|kv| kv == "p=title");
            let decoded = if meta.split(':').any(|kv| kv == "e=1") {
                b64_text(payload)
            } else {
                payload.to_string()
            };
            Some(if is_title {
                OscEvent::Notify {
                    title: Some(decoded),
                    body: String::new(),
                }
            } else {
                OscEvent::Notify {
                    title: None,
                    body: decoded,
                }
            })
        }
        "777" => {
            let mut it = arg.splitn(3, ';');
            if it.next()? != "notify" {
                return None;
            }
            let title = it.next().map(str::to_string).filter(|s| !s.is_empty());
            let body = it.next().unwrap_or("").to_string();
            Some(OscEvent::Notify { title, body })
        }
        "133" => {
            let mut it = arg.split(';');
            let mark = it.next()?.chars().next()?;
            let exit = if mark == 'D' {
                it.next().and_then(|s| s.trim().parse().ok())
            } else {
                None
            };
            Some(OscEvent::Prompt { mark, exit })
        }
        "1337" => {
            if let Some(dir) = arg.strip_prefix("CurrentDir=") {
                return Some(OscEvent::Cwd(dir.to_string()));
            }
            if let Some(kv) = arg.strip_prefix("SetUserVar=") {
                let (name, value) = kv.split_once('=')?;
                return Some(OscEvent::UserVar {
                    name: name.to_string(),
                    value: b64_text(value),
                });
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_split_across_reads() {
        let mut s = Sniffer::new();
        assert!(s.feed(b"hello \x1b]0;\xe2\x9c\xb3 Cla").is_empty());
        assert!(s.feed(b"ude Co").is_empty());
        let ev = s.feed(b"de\x07 after");
        assert_eq!(ev, vec![OscEvent::Title("✳ Claude Code".into())]);
        // Nothing left over.
        assert!(s.feed(b"plain").is_empty());
    }

    #[test]
    fn st_terminator_split_between_esc_and_backslash() {
        let mut s = Sniffer::new();
        assert!(s.feed(b"\x1b]9;Build done\x1b").is_empty());
        let ev = s.feed(b"\\x");
        assert_eq!(
            ev,
            vec![OscEvent::Notify {
                title: None,
                body: "Build done".into()
            }]
        );
    }

    #[test]
    fn progress_prompt_cwd_bell() {
        let mut s = Sniffer::new();
        let ev = s.feed(
            b"\x1b]9;4;3;\x07\x1b]9;4;1;42\x1b\\\x1b]133;D;2\x07\x1b]7;file://mac/Users/t%20o\x07\x07",
        );
        assert_eq!(
            ev,
            vec![
                OscEvent::Progress {
                    state: 3,
                    value: None
                },
                OscEvent::Progress {
                    state: 1,
                    value: Some(42)
                },
                OscEvent::Prompt {
                    mark: 'D',
                    exit: Some(2)
                },
                OscEvent::Cwd("/Users/t o".into()),
                OscEvent::Bell,
            ]
        );
    }

    #[test]
    fn notify_777_and_bel_inside_osc_is_not_a_bell() {
        let mut s = Sniffer::new();
        let ev = s.feed(b"\x1b]777;notify;Codex;Approval needed\x07");
        assert_eq!(
            ev,
            vec![OscEvent::Notify {
                title: Some("Codex".into()),
                body: "Approval needed".into()
            }]
        );
    }
}
