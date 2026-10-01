use super::session::sanitize::{tokenize, Token};

const CARRY_MAX: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum OscEvent {
    Title(String),
    Cwd(String),
    Notify { title: Option<String>, body: String },
    Progress { state: u8, value: Option<u8> },
    Prompt { mark: char, exit: Option<i32> },
    UserVar { name: String, value: String },
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
    let path = &rest[rest.find('/')?..];
    let mut p = percent_decode(path);
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
