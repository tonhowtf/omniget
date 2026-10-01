use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct SilenceSpan {
    pub start: f64,
    pub end: f64,
}

impl SilenceSpan {
    pub fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SilenceMap {
    #[serde(default)]
    pub version: u32,
    pub media_duration_secs: f64,
    #[serde(default)]
    pub spans: Vec<SilenceSpan>,
}

pub const CURRENT_VERSION: u32 = 1;

pub const PLAYBACK_PADDING_SECS: f64 = 0.15;

pub const MIN_SKIPPABLE_SECS: f64 = 0.4;

pub fn parse_spans(stderr: &str) -> Vec<SilenceSpan> {
    let mut spans = Vec::new();
    let mut open: Option<f64> = None;

    for line in stderr.lines() {
        if let Some(v) = field_after(line, "silence_start:") {
            open = Some(v);
        } else if let Some(end) = field_after(line, "silence_end:") {
            if let Some(start) = open.take() {
                if end > start {
                    spans.push(SilenceSpan { start, end });
                }
            }
        }
    }
    spans
}

fn field_after(line: &str, key: &str) -> Option<f64> {
    let idx = line.find(key)?;
    line[idx + key.len()..]
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

pub fn savings_secs(map: &SilenceMap) -> f64 {
    map.spans
        .iter()
        .filter(|s| s.duration() >= MIN_SKIPPABLE_SECS)
        .map(|s| (s.duration() - PLAYBACK_PADDING_SECS).max(0.0))
        .sum()
}

pub fn needs_recompute(map: &SilenceMap) -> bool {
    map.version != CURRENT_VERSION
}
