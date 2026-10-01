//! The splash `omniget claude` prints before Claude Code opens: Loop, the
//! OmniGet mascot, in its own colours, and who is about to run. Claude Code's
//! own header still follows below it: this is Claude Code, launched by OmniGet.

use std::io::IsTerminal;

// Loop, the OmniGet mascot, as a pixel grid (Higgsfield pixel art of static/loop.png,
// reduced to the mascot palette). One char per pixel; `.` is transparent.
const LOOP_PALETTE: &[(u8, u8, u8)] = &[
    (17, 17, 17),
    (60, 35, 38),
    (94, 64, 67),
    (29, 90, 60),
    (0, 0, 0),
    (74, 168, 75),
    (230, 165, 116),
    (253, 230, 200),
    (224, 58, 0),
    (245, 158, 53),
    (166, 30, 48),
    (110, 20, 40),
    (201, 115, 137),
];
const LOOP_PIXELS: &[&str] = &[
    "............................aaaabaaa",
    "..........................aaaabcccaa",
    ".........................aaaabccccaa",
    "aaa.....................aaaabbccccaa",
    "aaaaaaa...............aaaaaabbccccaa",
    "abaaaaaaa......ddddddddaaaaabcccccaa",
    "accbbeaaaaa.fffddddddddddddabccccaaa",
    "acccbeaaaaddfffddddddddddddddbcccaaa",
    "accccbbaaddddgghhhhhhhhdddddddcccaaa",
    "aaccccbbdddaggghhhhhhhhhhhddddddcaaa",
    "aacccccdddhgggghhhhhhhhhhhhhdddddaaa",
    ".aacccdddhhgaaaaaaaaahhhhhhhhddddaaa",
    ".aacccddhhhaaaaaaaaaaaahhhhhhhhddda.",
    ".aaacddhhhaaaaaaaaaaaaaaahhhhhhhddd.",
    "..aaadhhhaaaaaaaaaaaaaaaaaahhhhhhddd",
    "..aaddhhaaaaaaaaaaaaaaaaaaaahhhhhhdd",
    "..aadhhaaaaaaaaaaaaaaiiiaaaaahhhhhhd",
    "...adhaaaaaaaaaaaaaajiiiiaaaaahhhhhh",
    "...dhhaaaaaaaaaaaaajjaaaaiaaaaaahhhh",
    "...dhaaaiijjaaaaaaajaaaaaaaaaaaahhhh",
    "...ahaaaiijjjaaaaajjaaaaaaaaaaahhhh.",
    "..dhhaaiaaaajaaaaajaaaaaaaaaaahhhha.",
    "..ahaaaaaaaaajaaaaaaaaaaaaaaaahhha..",
    "..dhaaaaaaaaaaaaaaaaaaaaaaaaahhhaaka",
    "..dhaaaaaaaaaaaaaaaaaaaaaaaahhhallka",
    "..dhhaaaaaaaaaaaacmmmaaaaaahhhlklll.",
    "..hhhhhaaaaaaaaacmmmaaaaaahhakklkklk",
    "...ahhhhhaaaaaaaammaaaaahhhlkklkkklk",
    ".....ahhhhaaaaaaaaaaaaahhkkkllkkke..",
    ".......ahhhhhaaaaaaaaekkkkklkkkkii..",
    "........kkajhhhhhallkkkkkkkkkkkiidd.",
    "........akkjjkkkkkkkkkkkkkkkkkiiiddd",
    "........illjjkkkkkkkkklkkkkkkdiidddd",
    "........iilljkkkkkkkkkkkkkkadiiidddd",
    ".......diiajjjlkkkkkkkkkkkdddiiddddd",
    ".......diidddkkkkkkkkkkkdddddiiddddd",
    "......addiiddddkkkkkkkdddddddiiddddd",
    ".....adddiiddddddddddddddddddiiddddd",
    ".....ddddiiddddddddddddddddddiiddddd",
    "....addddiiidddddddddddddddddiiddddd",
];

/// OmniGet orange, Claude's coral, and a dim grey for the details.
const OMNIGET_ORANGE: (u8, u8, u8) = (255, 149, 0);
const CLAUDE_CORAL: (u8, u8, u8) = (217, 119, 87);
const DIM: (u8, u8, u8) = (150, 150, 150);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Depth {
    TrueColor,
    Ansi256,
}

impl Depth {
    pub fn detect() -> Depth {
        let ct = std::env::var("COLORTERM")
            .unwrap_or_default()
            .to_lowercase();
        if ct.contains("truecolor") || ct.contains("24bit") {
            Depth::TrueColor
        } else {
            Depth::Ansi256
        }
    }
}

/// Nearest xterm-256 colour (6x6x6 cube or the grey ramp).
pub fn to_256((r, g, b): (u8, u8, u8)) -> u8 {
    let level = |v: u8| -> u8 {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            (v - 35) / 40
        }
    };
    let (lr, lg, lb) = (level(r), level(g), level(b));
    let cube = 16 + 36 * lr + 6 * lg + lb;
    let steps = [0u8, 95, 135, 175, 215, 255];
    let cube_rgb = (steps[lr as usize], steps[lg as usize], steps[lb as usize]);
    let grey = ((r as u16 + g as u16 + b as u16) / 3) as u8;
    let gi = if grey > 238 {
        23
    } else {
        grey.saturating_sub(3) / 10
    };
    let grey_v = 8 + 10 * gi;
    let d = |a: (u8, u8, u8)| {
        let f = |x: u8, y: u8| (x as i32 - y as i32).pow(2);
        f(a.0, r) + f(a.1, g) + f(a.2, b)
    };
    if d((grey_v, grey_v, grey_v)) < d(cube_rgb) {
        232 + gi
    } else {
        cube
    }
}

fn fg(c: (u8, u8, u8), depth: Depth) -> String {
    match depth {
        Depth::TrueColor => format!("\x1b[38;2;{};{};{}m", c.0, c.1, c.2),
        Depth::Ansi256 => format!("\x1b[38;5;{}m", to_256(c)),
    }
}

fn bg(c: (u8, u8, u8), depth: Depth) -> String {
    match depth {
        Depth::TrueColor => format!("\x1b[48;2;{};{};{}m", c.0, c.1, c.2),
        Depth::Ansi256 => format!("\x1b[48;5;{}m", to_256(c)),
    }
}

fn pixel(row: &str, x: usize) -> Option<(u8, u8, u8)> {
    let ch = *row.as_bytes().get(x)?;
    if ch == b'.' {
        return None;
    }
    LOOP_PALETTE.get((ch - b'a') as usize).copied()
}

/// The mascot as terminal lines: two pixel rows per line with `▀`/`▄`.
pub fn art_lines(depth: Depth) -> Vec<String> {
    let width = LOOP_PIXELS.first().map_or(0, |r| r.len());
    let mut out = Vec::new();
    for pair in LOOP_PIXELS.chunks(2) {
        let (top, bottom) = (pair[0], pair.get(1).copied().unwrap_or(""));
        let mut line = String::new();
        for x in 0..width {
            match (pixel(top, x), pixel(bottom, x)) {
                (None, None) => line.push_str("\x1b[0m "),
                (Some(t), None) => line.push_str(&format!("\x1b[0m{}▀", fg(t, depth))),
                (None, Some(b)) => line.push_str(&format!("\x1b[0m{}▄", fg(b, depth))),
                (Some(t), Some(b)) => line.push_str(&format!("{}{}▀", fg(t, depth), bg(b, depth))),
            }
        }
        line.push_str("\x1b[0m");
        out.push(line);
    }
    out
}

/// Text beside the mascot. `version` is Claude Code's own (`claude --version`),
/// shown as is so an update never looks like a different product.
pub fn text_lines(
    version: Option<&str>,
    account: &str,
    who: Option<&str>,
    cwd: &str,
    depth: Depth,
) -> Vec<String> {
    let bold = "\x1b[1m";
    let reset = "\x1b[0m";
    let dim = fg(DIM, depth);
    let mut title = format!(
        "{bold}{}OmniGet{reset}{dim} with {reset}{bold}{}Claude Code{reset}",
        fg(OMNIGET_ORANGE, depth),
        fg(CLAUDE_CORAL, depth)
    );
    if let Some(v) = version {
        title.push_str(&format!(" {dim}v{v}{reset}"));
    }
    let mut lines = vec![
        title,
        String::new(),
        format!("{dim}conta{reset}  {account}"),
    ];
    if let Some(w) = who {
        lines.push(format!("{dim}login{reset}  {w}"));
    }
    lines.push(format!("{dim}pasta{reset}  {cwd}"));
    lines
}

/// Prints the splash when stdout is a terminal wide enough for it.
pub fn print(version: Option<&str>, account: &str, who: Option<&str>) {
    if !std::io::stdout().is_terminal() {
        return;
    }
    let depth = Depth::detect();
    let cwd = std::env::current_dir()
        .map(|p| {
            let s = p.display().to_string();
            match dirs_home() {
                Some(h) if s.starts_with(&h) => format!("~{}", &s[h.len()..]),
                _ => s,
            }
        })
        .unwrap_or_default();
    let art = art_lines(depth);
    let text = text_lines(version, account, who, &cwd, depth);
    let cols = terminal_cols();
    let art_width = LOOP_PIXELS.first().map_or(0, |r| r.len());
    let mut out = String::from("\n");
    if cols.is_some_and(|c| c < art_width + 40) {
        // Narrow terminal: mascot, then the text under it.
        for l in &art {
            out.push_str(&format!("  {l}\n"));
        }
        out.push('\n');
        for t in &text {
            out.push_str(&format!("  {t}\n"));
        }
    } else {
        let start = art.len().saturating_sub(text.len()) / 2;
        for (i, l) in art.iter().enumerate() {
            let t = i
                .checked_sub(start)
                .and_then(|j| text.get(j))
                .map(String::as_str)
                .unwrap_or("");
            out.push_str(&format!("  {l}   {t}\n"));
        }
    }
    out.push('\n');
    print!("{out}");
}

fn dirs_home() -> Option<String> {
    std::env::var("HOME").ok().filter(|h| !h.is_empty())
}

fn terminal_cols() -> Option<usize> {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.parse().ok())
        .or_else(|| {
            std::process::Command::new("tput")
                .arg("cols")
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        })
}
