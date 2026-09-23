//! Makes a raw PTY transcript safe to replay into a fresh emulator.
//!
//! A transcript is what the program wrote, not what the screen showed. Played
//! back as is it would:
//! - answer terminal queries a second time (DA, DSR, DECRQM, XTVERSION, OSC
//!   colour queries, DECRQSS/XTGETTCAP), typing the answers into the shell;
//! - touch the clipboard again (OSC 52) and ring every bell ever rung;
//! - flash every full-screen program that ran and exited (alternate screen);
//! - leave input modes (mouse, bracketed paste, focus, application cursor)
//!   in whatever order the toggles happened to be cut by the history cap.
//!
//! [`sanitize_for_replay`] drops the queries, the clipboard, bells and the
//! alternate-screen content of programs that already exited, strips every
//! input-mode toggle and re-emits only the final state of each mode at the end.
//! A full-screen program still running keeps its alternate screen, so the live
//! output that follows lands where it expects.

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

/// DEC private modes that select the alternate screen.
const ALT_MODES: [u32; 3] = [47, 1047, 1049];
/// DEC private modes that change how input is reported. Only the final state
/// of each is replayed.
const INPUT_MODES: [u32; 9] = [1, 1000, 1002, 1003, 1004, 1005, 1006, 1015, 2004];
/// Cursor visibility: final state only.
const CURSOR_MODE: u32 = 25;
/// Synchronized output: transient by definition, never replayed.
const SYNC_MODE: u32 = 2026;

#[derive(Debug, PartialEq)]
enum Token<'a> {
    Text(&'a [u8]),
    /// A single C0 control byte outside any sequence.
    Ctl(u8),
    Csi(&'a [u8]),
    Osc(&'a [u8]),
    /// DCS, SOS, PM, APC: `ESC P|X|^|_ … ST`.
    Str(u8, &'a [u8]),
    /// `ESC` plus intermediates and a final byte.
    Esc(&'a [u8]),
    /// A sequence the transcript ends in the middle of.
    Partial(&'a [u8]),
}

fn tokenize(input: &[u8]) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    let n = input.len();
    while i < n {
        let b = input[i];
        if b == ESC {
            let start = i;
            if i + 1 >= n {
                out.push(Token::Partial(&input[start..]));
                break;
            }
            let k = input[i + 1];
            match k {
                b'[' => {
                    let mut j = i + 2;
                    let mut done = false;
                    while j < n {
                        let c = input[j];
                        if (0x40..=0x7e).contains(&c) {
                            done = true;
                            break;
                        }
                        if c == ESC || c == 0x18 || c == 0x1a {
                            break;
                        }
                        j += 1;
                    }
                    if done {
                        out.push(Token::Csi(&input[start..=j]));
                        i = j + 1;
                    } else if j >= n {
                        out.push(Token::Partial(&input[start..]));
                        break;
                    } else {
                        // Cancelled (CAN/SUB) or interrupted by a new ESC.
                        if input[j] != ESC {
                            j += 1;
                        }
                        i = j;
                    }
                }
                b']' | b'P' | b'X' | b'^' | b'_' => {
                    let mut j = i + 2;
                    let mut end = None;
                    while j < n {
                        let c = input[j];
                        if c == BEL && k == b']' {
                            end = Some(j + 1);
                            break;
                        }
                        if c == ESC && j + 1 < n && input[j + 1] == b'\\' {
                            end = Some(j + 2);
                            break;
                        }
                        if c == 0x18 || c == 0x1a {
                            end = Some(j + 1);
                            break;
                        }
                        j += 1;
                    }
                    match end {
                        Some(e) => {
                            if k == b']' {
                                out.push(Token::Osc(&input[start..e]));
                            } else {
                                out.push(Token::Str(k, &input[start..e]));
                            }
                            i = e;
                        }
                        None => {
                            out.push(Token::Partial(&input[start..]));
                            break;
                        }
                    }
                }
                _ => {
                    let mut j = i + 1;
                    while j < n && (0x20..=0x2f).contains(&input[j]) {
                        j += 1;
                    }
                    if j >= n {
                        out.push(Token::Partial(&input[start..]));
                        break;
                    }
                    out.push(Token::Esc(&input[start..=j]));
                    i = j + 1;
                }
            }
        } else if b < 0x20 && b != b'\n' && b != b'\r' && b != b'\t' && b != 0x08 {
            out.push(Token::Ctl(b));
            i += 1;
        } else {
            let start = i;
            while i < n {
                let c = input[i];
                if c == ESC || (c < 0x20 && c != b'\n' && c != b'\r' && c != b'\t' && c != 0x08) {
                    break;
                }
                i += 1;
            }
            out.push(Token::Text(&input[start..i]));
        }
    }
    out
}

/// Splits `ESC [ <prefix> params <intermediates> final`.
struct Csi<'a> {
    prefix: Option<u8>,
    params: &'a [u8],
    inter: &'a [u8],
    fin: u8,
}

fn parse_csi(seq: &[u8]) -> Csi<'_> {
    let body = &seq[2..seq.len() - 1];
    let fin = seq[seq.len() - 1];
    let (prefix, rest) = match body.first() {
        Some(&c) if matches!(c, b'?' | b'>' | b'<' | b'=') => (Some(c), &body[1..]),
        _ => (None, body),
    };
    let split = rest
        .iter()
        .position(|c| (0x20..=0x2f).contains(c))
        .unwrap_or(rest.len());
    Csi {
        prefix,
        params: &rest[..split],
        inter: &rest[split..],
        fin,
    }
}

fn param_list(params: &[u8]) -> Vec<Option<u32>> {
    if params.is_empty() {
        return Vec::new();
    }
    params
        .split(|&c| c == b';')
        .map(|p| std::str::from_utf8(p).ok().and_then(|s| s.parse().ok()))
        .collect()
}

/// True for control sequences that make the terminal send something back.
fn csi_is_query(c: &Csi<'_>) -> bool {
    match (c.prefix, c.inter, c.fin) {
        // DA1/DA2/DA3.
        (_, b"", b'c') => true,
        // DSR / DECDSR.
        (None | Some(b'?'), b"", b'n') => true,
        // Window ops: reports, and resizes/moves that make no sense on replay.
        (None, b"", b't') => true,
        // DECRQM.
        (_, b"$", b'p') => true,
        // XTVERSION.
        (Some(b'>'), b"", b'q') => true,
        // Kitty keyboard protocol query.
        (Some(b'?'), b"", b'u') => true,
        // DECREQTPARM.
        (None, b"", b'x') => true,
        // XTQMODKEYS.
        (Some(b'?'), b"", b'm') => true,
        _ => false,
    }
}

#[derive(Default)]
struct Modes {
    /// Last state seen for each input mode, in first-seen order.
    input: Vec<(u32, bool)>,
    cursor_hidden: Option<bool>,
}

impl Modes {
    fn set(&mut self, mode: u32, on: bool) {
        if let Some(slot) = self.input.iter_mut().find(|(m, _)| *m == mode) {
            slot.1 = on;
        } else {
            self.input.push((mode, on));
        }
    }
}

struct Replay {
    main: Vec<u8>,
    /// Content of the alternate screen currently open, with the mode that
    /// opened it.
    alt: Option<(u32, Vec<u8>)>,
    modes: Modes,
}

impl Replay {
    fn sink(&mut self) -> &mut Vec<u8> {
        match self.alt.as_mut() {
            Some((_, buf)) => buf,
            None => &mut self.main,
        }
    }

    fn private_modes(&mut self, seq: &[u8], c: &Csi<'_>) {
        let on = c.fin == b'h';
        let mut kept: Vec<String> = Vec::new();
        let params = param_list(c.params);
        if params.is_empty() {
            self.sink().extend_from_slice(seq);
            return;
        }
        for p in params {
            let Some(p) = p else { continue };
            if ALT_MODES.contains(&p) {
                // Flush what was kept so far so ordering survives.
                self.flush_kept(&mut kept, c.fin);
                if on {
                    if self.alt.is_none() {
                        self.alt = Some((p, Vec::new()));
                    }
                } else if self.alt.is_some() {
                    // The program exited: its screen is gone on a real terminal too.
                    self.alt = None;
                } else {
                    // The history starts inside a full-screen program: everything
                    // before this exit was its screen.
                    self.main.clear();
                }
            } else if INPUT_MODES.contains(&p) {
                self.modes.set(p, on);
            } else if p == CURSOR_MODE {
                self.modes.cursor_hidden = Some(!on);
            } else if p == SYNC_MODE {
            } else {
                kept.push(p.to_string());
            }
        }
        self.flush_kept(&mut kept, c.fin);
    }

    fn flush_kept(&mut self, kept: &mut Vec<String>, fin: u8) {
        if kept.is_empty() {
            return;
        }
        let seq = format!("\x1b[?{}{}", kept.join(";"), fin as char);
        self.sink().extend_from_slice(seq.as_bytes());
        kept.clear();
    }
}

fn osc_is_dangerous(seq: &[u8]) -> bool {
    // Strip `ESC ]` and the terminator.
    let body = &seq[2..];
    let body = body
        .strip_suffix(b"\x1b\\")
        .or_else(|| body.strip_suffix(&[BEL]))
        .unwrap_or(body);
    let num_end = body.iter().position(|&c| c == b';').unwrap_or(body.len());
    let num: Option<u32> = std::str::from_utf8(&body[..num_end])
        .ok()
        .and_then(|s| s.parse().ok());
    match num {
        // Clipboard: never on replay.
        Some(52) => true,
        // Colour/palette queries answer back.
        Some(4 | 5 | 10..=19 | 104 | 105 | 110..=119) => body[num_end..].contains(&b'?'),
        _ => false,
    }
}

fn str_is_query(kind: u8, seq: &[u8]) -> bool {
    match kind {
        // DECRQSS (`$q`) and XTGETTCAP (`+q`).
        b'P' => {
            let body = &seq[2..];
            let body = body
                .iter()
                .skip_while(|c| c.is_ascii_digit() || **c == b';');
            let head: Vec<u8> = body.take(2).copied().collect();
            head == b"$q" || head == b"+q"
        }
        // APC (kitty graphics, queries included), PM and SOS: nothing to replay.
        _ => true,
    }
}

/// Returns a copy of `input` that is safe to write into a fresh emulator.
pub fn sanitize_for_replay(input: &[u8]) -> Vec<u8> {
    let mut r = Replay {
        main: Vec::with_capacity(input.len()),
        alt: None,
        modes: Modes::default(),
    };
    for tok in tokenize(input) {
        match tok {
            Token::Text(t) => r.sink().extend_from_slice(t),
            // Bells and ENQ (answerback) are dropped; other C0 bytes kept.
            Token::Ctl(BEL) | Token::Ctl(0x05) => {}
            Token::Ctl(b) => r.sink().push(b),
            Token::Csi(seq) => {
                let c = parse_csi(seq);
                if csi_is_query(&c) {
                    continue;
                }
                if c.prefix == Some(b'?') && c.inter.is_empty() && (c.fin == b'h' || c.fin == b'l')
                {
                    r.private_modes(seq, &c);
                    continue;
                }
                r.sink().extend_from_slice(seq);
            }
            Token::Osc(seq) => {
                if !osc_is_dangerous(seq) {
                    r.sink().extend_from_slice(seq);
                }
            }
            Token::Str(kind, seq) => {
                if !str_is_query(kind, seq) {
                    r.sink().extend_from_slice(seq);
                }
            }
            Token::Esc(seq) => {
                if seq == b"\x1bc" {
                    // RIS wipes everything before it.
                    r.main.clear();
                    r.alt = None;
                    r.modes = Modes::default();
                } else {
                    r.sink().extend_from_slice(seq);
                }
            }
            // The live stream finishes it.
            Token::Partial(seq) => {
                // Kept raw at the very end, after the mode restore below.
                let mut out = finish(r);
                out.extend_from_slice(seq);
                return out;
            }
        }
    }
    finish(r)
}

fn finish(r: Replay) -> Vec<u8> {
    let Replay {
        mut main,
        alt,
        modes,
    } = r;
    if let Some((mode, buf)) = alt {
        main.extend_from_slice(format!("\x1b[?{mode}h").as_bytes());
        main.extend_from_slice(&buf);
    }
    for (mode, on) in modes.input {
        if on {
            main.extend_from_slice(format!("\x1b[?{mode}h").as_bytes());
        }
    }
    if modes.cursor_hidden == Some(true) {
        main.extend_from_slice(b"\x1b[?25l");
    }
    main
}

#[cfg(test)]
mod tests {
    use super::sanitize_for_replay as s;

    #[test]
    fn plain_text_and_colours_pass_through() {
        let input = b"hello \x1b[31mred\x1b[0m\r\nline 2\r\n";
        assert_eq!(s(input), input.to_vec());
    }

    #[test]
    fn queries_are_dropped() {
        let input =
            b"a\x1b[c\x1b[>c\x1b[6n\x1b[?6n\x1b[18t\x1b[?2004$p\x1b[>q\x1b]11;?\x07\x1bP$qm\x1b\\b";
        assert_eq!(s(input), b"ab".to_vec());
    }

    #[test]
    fn clipboard_and_bells_are_dropped_titles_kept() {
        let input = b"\x07x\x1b]52;c;aGk=\x07\x1b]0;title\x07y";
        assert_eq!(s(input), b"x\x1b]0;title\x07y".to_vec());
    }

    #[test]
    fn closed_alt_screen_disappears() {
        let input = b"before\r\n\x1b[?1049hvim screen\x1b[?1049lafter";
        assert_eq!(s(input), b"before\r\nafter".to_vec());
    }

    #[test]
    fn open_alt_screen_is_kept() {
        let input = b"before\r\n\x1b[?1049h\x1b[?1000hhtop";
        assert_eq!(s(input), b"before\r\n\x1b[?1049hhtop\x1b[?1000h".to_vec());
    }

    #[test]
    fn unmatched_alt_exit_drops_what_came_before() {
        let input = b"tui garbage\x1b[?1049lprompt$ ";
        assert_eq!(s(input), b"prompt$ ".to_vec());
    }

    #[test]
    fn only_final_input_modes_survive() {
        let input = b"\x1b[?2004h$ ls\x1b[?2004l\r\nout\r\n\x1b[?2004h$ \x1b[?25l\x1b[?25h";
        assert_eq!(s(input), b"$ ls\r\nout\r\n$ \x1b[?2004h".to_vec());
    }

    #[test]
    fn mixed_private_modes_keep_the_harmless_ones() {
        let input = b"\x1b[?7;2004h";
        assert_eq!(s(input), b"\x1b[?7h\x1b[?2004h".to_vec());
    }

    #[test]
    fn ris_wipes_the_past() {
        assert_eq!(s(b"old\x1bcnew"), b"new".to_vec());
    }

    #[test]
    fn trailing_partial_sequence_stays_raw() {
        assert_eq!(s(b"x\x1b[3"), b"x\x1b[3".to_vec());
        assert_eq!(s(b"x\x1b]0;ti"), b"x\x1b]0;ti".to_vec());
    }

    #[test]
    fn utf8_is_untouched() {
        let input = "ação ✓ 日本\r\n".as_bytes();
        assert_eq!(s(input), input.to_vec());
    }
}
