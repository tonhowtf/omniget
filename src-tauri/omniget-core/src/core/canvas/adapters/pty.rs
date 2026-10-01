pub const USER_TYPING_HOLD_MS: u64 = 3_000;
pub const ECHO_MIN_MS: u64 = 120;
pub const ECHO_MAX_MS: u64 = 1_500;
pub const CONFIRM_MS: u64 = 3_000;
pub const RETRY_GAP_MS: u64 = 1_000;
pub const MAX_ENTER_RETRIES: u32 = 2;
pub const GATE_TIMEOUT_MS: u64 = 30 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    Taken,
    Wait,
    Retry,
    Stuck,
}

pub fn confirm_step(elapsed_ms: u64, retries: u32, taken: bool) -> Confirm {
    if taken {
        return Confirm::Taken;
    }
    let window = if retries == 0 {
        CONFIRM_MS
    } else {
        RETRY_GAP_MS
    };
    if elapsed_ms < window {
        return Confirm::Wait;
    }
    if retries < MAX_ENTER_RETRIES {
        Confirm::Retry
    } else {
        Confirm::Stuck
    }
}

pub fn echoed(before: &str, now: &str, pasted: &str) -> bool {
    if before == now {
        return false;
    }
    let tail: String = pasted
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .chars()
        .rev()
        .take(24)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let lower = now.to_ascii_lowercase();
    tail.is_empty()
        || now.contains(&tail)
        || lower.contains("pasted text")
        || lower.contains("[pasted")
}

pub fn descendants_now(root: u32) -> std::collections::HashSet<u32> {
    #[cfg(unix)]
    {
        let out = crate::core::process::std_command("ps")
            .args(["-A", "-o", "pid=", "-o", "ppid="])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
        let Ok(out) = out else {
            return Default::default();
        };
        let table = crate::core::llm::drivers::codex::supervisor::parse_ps(
            &String::from_utf8_lossy(&out.stdout),
        );
        crate::core::llm::drivers::codex::supervisor::descendants(&table, root)
            .into_iter()
            .collect()
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        Default::default()
    }
}

pub fn working_children(
    now: &std::collections::HashSet<u32>,
    before: &std::collections::HashSet<u32>,
) -> bool {
    now.iter().any(|p| !before.contains(p))
}

pub fn flatten(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn decode_escapes(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' || i + 1 >= b.len() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        match b[i + 1] {
            b'n' => out.push(b'\r'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'e' | b'E' => out.push(0x1b),
            b'\\' => out.push(b'\\'),
            b'x' => {
                let hex = b
                    .get(i + 2..i + 4)
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 4;
                        continue;
                    }
                    None => {
                        out.push(b'\\');
                        out.push(b'x');
                    }
                }
            }
            other => {
                out.push(b'\\');
                out.push(other);
            }
        }
        i += 2;
    }
    out
}

pub fn decode_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.peek() {
            Some('n') => {
                it.next();
                out.push('\n');
            }
            Some('t') => {
                it.next();
                out.push('\t');
            }
            Some('\\') => {
                it.next();
                out.push('\\');
                if let Some(&n) = it.peek() {
                    if n == 'n' || n == 't' {
                        it.next();
                        out.push(n);
                    }
                }
            }
            _ => out.push('\\'),
        }
    }
    out
}
