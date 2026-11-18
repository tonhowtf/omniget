use serde::{Deserialize, Serialize};

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
}
