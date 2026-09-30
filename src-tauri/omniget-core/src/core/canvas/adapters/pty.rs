//! Delivery into a terminal (estudo 98/03 §B.8), the default for any CLI.
//!
//! 1. **Gate:** the target is ready for 250 ms (or `needs_you` for a reply to
//!    its own question) and the user has not typed in it in the last 3 s.
//!    Focusing the node cancels nothing; only typing holds the delivery.
//! 2. **Paste:** bracketed when the program turned mode 2004 on.
//! 3. **Enter apart:** after the paste echoes on the screen (120 ms minimum,
//!    1.5 s maximum), `\r` outside the paste block.
//! 4. **Confirm:** up to 3 s waiting for the prompt to be taken
//!    (`UserPromptSubmit`/`BeforeAgent`, or the agent going busy). Without a
//!    sign, Enter again, twice at most, 1 s apart; then `stuck`.
//!
//! This module holds the timings and the decisions; the app runs the loop.

pub const USER_TYPING_HOLD_MS: u64 = 3_000;
pub const ECHO_MIN_MS: u64 = 120;
pub const ECHO_MAX_MS: u64 = 1_500;
pub const CONFIRM_MS: u64 = 3_000;
pub const RETRY_GAP_MS: u64 = 1_000;
pub const MAX_ENTER_RETRIES: u32 = 2;
/// A queued message waits this long for its gate before failing.
pub const GATE_TIMEOUT_MS: u64 = 30 * 60 * 1000;

/// What to do after the Enter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    /// The prompt was taken.
    Taken,
    /// Keep waiting.
    Wait,
    /// Press Enter again.
    Retry,
    /// Give up: `delivery_stuck`.
    Stuck,
}

/// `elapsed_ms` since the last Enter, `retries` Enters already repeated,
/// `taken` = a hook said the prompt was submitted or the agent went busy.
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

/// The paste has echoed: the screen changed since the paste and the tail of
/// the pasted text (or the "pasted text" placeholder of the TUI) shows.
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

/// Every process under `root` right now (`ps`), for a program without turn
/// hooks: while the child it started for a prompt lives (`sleep`, a build),
/// its turn is not over even when the screen stays quiet. Empty on Windows
/// or when `ps` fails.
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

/// The program still works on a prompt: a process started under it after
/// the delivery (`before` = what already ran then, helpers that live for
/// the whole session) is alive.
pub fn working_children(
    now: &std::collections::HashSet<u32>,
    before: &std::collections::HashSet<u32>,
) -> bool {
    now.iter().any(|p| !before.contains(p))
}

/// One line, for a program without bracketed paste (a newline would be Enter).
pub fn flatten(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Decodes the escapes of `ask --raw`: `\n` Enter (CR), `\r`, `\t`, `\e` ESC,
/// `\xNN`, `\\`. Anything else is kept as written.
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

/// Decodes the escapes of a prompt argument: `\n` newline, `\t` tab; `\\n`
/// stays a literal backslash-n.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_is_retried_twice_then_stuck() {
        assert_eq!(confirm_step(100, 0, false), Confirm::Wait);
        assert_eq!(confirm_step(3_000, 0, false), Confirm::Retry);
        assert_eq!(confirm_step(500, 1, false), Confirm::Wait);
        assert_eq!(confirm_step(1_000, 1, false), Confirm::Retry);
        assert_eq!(confirm_step(1_000, 2, false), Confirm::Stuck);
        assert_eq!(confirm_step(1_000, 2, true), Confirm::Taken);
    }

    #[test]
    fn raw_escapes() {
        assert_eq!(decode_escapes(r"\e[A\n"), b"\x1b[A\r".to_vec());
        assert_eq!(decode_escapes(r"\x03"), vec![3]);
        assert_eq!(decode_escapes(r"a\\b"), b"a\\b".to_vec());
        assert_eq!(decode_escapes(r"\xZZ"), b"\\xZZ".to_vec());
        assert_eq!(decode_text(r"a\nb\\nc"), "a\nb\\nc");
    }

    #[test]
    fn only_children_started_after_the_delivery_count() {
        let before: std::collections::HashSet<u32> = [10, 11].into_iter().collect();
        let same: std::collections::HashSet<u32> = [10, 11].into_iter().collect();
        let more: std::collections::HashSet<u32> = [10, 11, 40].into_iter().collect();
        assert!(!working_children(&same, &before));
        assert!(working_children(&more, &before));
        assert!(!working_children(&Default::default(), &before));
    }

    #[cfg(unix)]
    #[test]
    fn a_running_child_is_seen_under_its_program() {
        let mut p = std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 3; true"])
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let kids = descendants_now(p.id());
        let _ = p.kill();
        let _ = p.wait();
        assert!(!kids.is_empty(), "no child seen under {}", p.id());
    }

    #[test]
    fn echo_is_seen_on_screen() {
        assert!(!echoed("same", "same", "hello"));
        assert!(echoed("> ", "> hello world", "hello world"));
        assert!(echoed("> ", "> [Pasted text #1 +40 lines]", "x"));
        assert_eq!(flatten("a\n\n b \nc"), "a b c");
    }
}
