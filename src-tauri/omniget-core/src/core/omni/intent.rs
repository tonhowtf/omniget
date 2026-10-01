//! Mascot `intent` (Phase 5). Owned by f5-omni-intent.
//!
//! The mascot never reacts to free text. An agent expresses itself through a
//! closed enum pair (`Animation`, `Mood`) plus an optional one-liner, emitted
//! inside the turn that is already streaming (an ```omni { ... }``` fenced
//! block, or a bare JSON object at the very end of the message). Anything that
//! does not parse falls back to `Idle`/`Neutral`: the parser never panics and
//! never invents an animation.

use serde::{Deserialize, Serialize};

/// Hard cap for `Intent::line`, in characters (not bytes).
pub const MAX_LINE_CHARS: usize = 80;

/// How many bytes from the end of the text the parser is willing to scan for a
/// trailing JSON object. Keeps `parse_intent` linear on a long answer.
const TAIL_SCAN_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Animation {
    #[default]
    Idle,
    Walk,
    Sit,
    Wave,
    Work,
    Sleep,
    Talk,
    Celebrate,
    Worried,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mood {
    #[default]
    Neutral,
    Happy,
    Curious,
    Proud,
    Worried,
    Sleepy,
    Focused,
}

impl Animation {
    pub const ALL: [Animation; 9] = [
        Animation::Idle,
        Animation::Walk,
        Animation::Sit,
        Animation::Wave,
        Animation::Work,
        Animation::Sleep,
        Animation::Talk,
        Animation::Celebrate,
        Animation::Worried,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Animation::Idle => "idle",
            Animation::Walk => "walk",
            Animation::Sit => "sit",
            Animation::Wave => "wave",
            Animation::Work => "work",
            Animation::Sleep => "sleep",
            Animation::Talk => "talk",
            Animation::Celebrate => "celebrate",
            Animation::Worried => "worried",
        }
    }

    /// Case-insensitive, trimmed. Unknown name -> `None` (caller falls back).
    pub fn parse(raw: &str) -> Option<Animation> {
        let raw = raw.trim();
        Animation::ALL
            .into_iter()
            .find(|a| a.as_str().eq_ignore_ascii_case(raw))
    }
}

impl Mood {
    pub const ALL: [Mood; 7] = [
        Mood::Neutral,
        Mood::Happy,
        Mood::Curious,
        Mood::Proud,
        Mood::Worried,
        Mood::Sleepy,
        Mood::Focused,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Mood::Neutral => "neutral",
            Mood::Happy => "happy",
            Mood::Curious => "curious",
            Mood::Proud => "proud",
            Mood::Worried => "worried",
            Mood::Sleepy => "sleepy",
            Mood::Focused => "focused",
        }
    }

    pub fn parse(raw: &str) -> Option<Mood> {
        let raw = raw.trim();
        Mood::ALL
            .into_iter()
            .find(|m| m.as_str().eq_ignore_ascii_case(raw))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intent {
    pub animation: Animation,
    pub mood: Mood,
    /// At most `MAX_LINE_CHARS` characters; `None` when the mascot stays quiet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
}

impl Default for Intent {
    fn default() -> Self {
        Intent::neutral()
    }
}

impl Intent {
    /// The fallback the whole module agrees on: idle body, neutral face, no line.
    pub const fn neutral() -> Self {
        Intent {
            animation: Animation::Idle,
            mood: Mood::Neutral,
            line: None,
        }
    }

    pub fn new(animation: Animation, mood: Mood) -> Self {
        Intent {
            animation,
            mood,
            line: None,
        }
    }

    /// Attaches a line, trimmed and truncated to `MAX_LINE_CHARS` characters.
    /// An empty (or whitespace-only) line is dropped.
    pub fn with_line(mut self, line: impl Into<String>) -> Self {
        self.line = normalise_line(&line.into());
        self
    }
}

/// Collapses whitespace, trims, truncates to `MAX_LINE_CHARS` chars.
/// Returns `None` for an empty result.
fn normalise_line(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len().min(MAX_LINE_CHARS * 4));
    let mut pending_space = false;
    let mut chars = 0usize;
    for ch in raw.chars() {
        if ch.is_whitespace() {
            if !out.is_empty() {
                pending_space = true;
            }
            continue;
        }
        if pending_space {
            if chars == MAX_LINE_CHARS {
                break;
            }
            out.push(' ');
            chars += 1;
            pending_space = false;
        }
        if chars == MAX_LINE_CHARS {
            break;
        }
        out.push(ch);
        chars += 1;
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Lenient shape of what a model may write. Every field is optional and typed
/// as a string so that a wrong enum value degrades instead of failing the whole
/// object.
#[derive(Debug, Deserialize)]
struct IntentDto {
    #[serde(default)]
    animation: Option<String>,
    #[serde(default)]
    mood: Option<String>,
    #[serde(default)]
    line: Option<String>,
}

/// Extracts the mascot intent from the text of a turn.
///
/// Looks for, in order: a fenced ```omni { ... }``` block (the last one wins),
/// then a bare JSON object closing the message. Malformed input, unknown enum
/// values, a missing block: all fall back to `Intent::neutral()`.
pub fn parse_intent(text: &str) -> Intent {
    if let Some(block) = last_omni_block(text) {
        if let Some(intent) = intent_from_json(block) {
            return intent;
        }
    }
    if let Some(tail) = trailing_json_object(text) {
        if let Some(intent) = intent_from_json(tail) {
            return intent;
        }
    }
    Intent::neutral()
}

fn intent_from_json(raw: &str) -> Option<Intent> {
    let dto: IntentDto = serde_json::from_str(raw.trim()).ok()?;
    let animation = dto
        .animation
        .as_deref()
        .and_then(Animation::parse)
        .unwrap_or_default();
    let mood = dto
        .mood
        .as_deref()
        .and_then(Mood::parse)
        .unwrap_or_default();
    let line = dto.line.as_deref().and_then(normalise_line);
    Some(Intent {
        animation,
        mood,
        line,
    })
}

/// Body of the last ```omni ... ``` fence in `text`, if any.
fn last_omni_block(text: &str) -> Option<&str> {
    let mut found = None;
    let mut cursor = 0usize;
    while let Some(rel) = text[cursor..].find("```") {
        let open = cursor + rel + 3;
        let rest = text.get(open..)?;
        // Language tag runs to the end of the line.
        let (tag, body_start) = match rest.find('\n') {
            Some(nl) => (&rest[..nl], open + nl + 1),
            None => return found,
        };
        let close_rel = match text.get(body_start..).and_then(|s| s.find("```")) {
            Some(i) => i,
            None => return found,
        };
        let body = &text[body_start..body_start + close_rel];
        if tag.trim().eq_ignore_ascii_case("omni") {
            found = Some(body);
        }
        cursor = body_start + close_rel + 3;
        if cursor >= text.len() {
            break;
        }
    }
    found
}

/// The JSON object that closes the message, scanning at most `TAIL_SCAN_BYTES`
/// bytes back from the end. Brace matching ignores braces inside strings.
fn trailing_json_object(text: &str) -> Option<&str> {
    let trimmed = text.trim_end();
    if !trimmed.ends_with('}') {
        return None;
    }
    let start_limit = trimmed
        .len()
        .saturating_sub(TAIL_SCAN_BYTES)
        .min(trimmed.len());
    let window_start = floor_char_boundary(trimmed, start_limit);
    let window = &trimmed[window_start..];
    let bytes = window.as_bytes();

    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    let mut open_at: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    open_at = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 && i + 1 == bytes.len() {
                    return open_at.map(|s| &window[s..]);
                }
                if depth < 0 {
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

fn floor_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}
