use serde::{Deserialize, Serialize};

pub const MAX_LINE_CHARS: usize = 80;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
}

impl Default for Intent {
    fn default() -> Self {
        Intent::neutral()
    }
}

impl Intent {
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

    pub fn with_line(mut self, line: impl Into<String>) -> Self {
        self.line = normalise_line(&line.into());
        self
    }
}

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
