//! Transcript segments and the formats they arrive in: the Teams transcript
//! JSON (`entries[]` with `speakerDisplayName`), WebVTT with `<v Speaker>`
//! voice tags (Teams, Vexa exports), plain SRT, and the two-track merge that
//! turns a mic recording and a system-audio recording into "You" / "Others".

use serde::{Deserialize, Serialize};

use crate::core::subtitle_merge::parse_cues;

pub const SPEAKER_ME: &str = "You";
pub const SPEAKER_OTHERS: &str = "Others";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    pub text: String,
}

/// Teams answers `temporaryDownloadUrl?format=json` with
/// `{"entries":[{"startOffset":"00:00:01.2340000","endOffset":...,
/// "speakerDisplayName":"Ana","text":"..."}]}`.
pub fn parse_teams_json(text: &str) -> Option<Vec<Segment>> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let entries = json.get("entries")?.as_array()?;
    let mut out = Vec::with_capacity(entries.len());
    for e in entries {
        let body = e["text"].as_str().unwrap_or("").trim();
        if body.is_empty() {
            continue;
        }
        let start = e["startOffset"]
            .as_str()
            .and_then(parse_offset)
            .unwrap_or(0);
        let end = e["endOffset"]
            .as_str()
            .and_then(parse_offset)
            .unwrap_or(start);
        let speaker = e["speakerDisplayName"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        out.push(Segment {
            start_ms: start,
            end_ms: end.max(start),
            speaker,
            text: body.to_string(),
        });
    }
    Some(out)
}

/// `HH:MM:SS.fffffff` (Teams writes seven fractional digits).
fn parse_offset(s: &str) -> Option<u64> {
    let mut parts = s.trim().split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let sec: f64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(((h * 3600.0 + m * 60.0 + sec) * 1000.0).round() as u64)
}

/// VTT or SRT. A `<v Name>` voice tag becomes the speaker; other tags go.
pub fn parse_subtitles(text: &str) -> Vec<Segment> {
    parse_cues(text)
        .into_iter()
        .filter_map(|c| {
            let (speaker, body) = split_voice(&c.text);
            let body = strip_tags(&body)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if body.is_empty() {
                return None;
            }
            Some(Segment {
                start_ms: c.start_ms,
                end_ms: c.end_ms.max(c.start_ms),
                speaker,
                text: body,
            })
        })
        .collect()
}

/// Any transcript file OmniGet accepts: Teams JSON first, then VTT/SRT.
pub fn parse_any(text: &str) -> Vec<Segment> {
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with('{') {
        if let Some(segs) = parse_teams_json(trimmed) {
            return segs;
        }
    }
    parse_subtitles(trimmed)
}

fn split_voice(text: &str) -> (Option<String>, String) {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("<v") {
        if let Some(close) = rest.find('>') {
            let name = rest[..close].trim();
            // `<v.loud Ana>`: classes come before the space.
            let name = match name.split_once(' ') {
                Some((cls, n)) if cls.starts_with('.') => n.trim(),
                _ => name,
            };
            let body = rest[close + 1..].replace("</v>", "");
            let speaker = Some(name.to_string()).filter(|n| !n.is_empty());
            return (speaker, body);
        }
    }
    (None, t.to_string())
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Joins consecutive segments from the same speaker when the gap is short,
/// so a Teams transcript (one entry per phrase) reads as turns.
pub fn merge_turns(segments: &[Segment], max_gap_ms: u64) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    for s in segments {
        if let Some(last) = out.last_mut() {
            if last.speaker == s.speaker
                && s.start_ms <= last.end_ms.saturating_add(max_gap_ms)
                && last.text.len() < 1200
            {
                last.text.push(' ');
                last.text.push_str(&s.text);
                last.end_ms = last.end_ms.max(s.end_ms);
                continue;
            }
        }
        out.push(s.clone());
    }
    out
}

/// Merges the mic track ("You") and the system track ("Others") by time.
/// Without headphones the mic also hears the call, so a mic segment that
/// overlaps a system segment saying mostly the same words is an echo and goes.
pub fn merge_tracks(mic: Vec<Segment>, system: Vec<Segment>) -> Vec<Segment> {
    let mic: Vec<Segment> = mic
        .into_iter()
        .filter(|m| !system.iter().any(|s| is_echo(m, s)))
        .map(|mut m| {
            m.speaker = Some(SPEAKER_ME.to_string());
            m
        })
        .collect();
    let mut all: Vec<Segment> = system
        .into_iter()
        .map(|mut s| {
            s.speaker = Some(SPEAKER_OTHERS.to_string());
            s
        })
        .chain(mic)
        .collect();
    all.sort_by_key(|s| (s.start_ms, s.end_ms));
    all
}

fn is_echo(mic: &Segment, sys: &Segment) -> bool {
    let overlap = mic.end_ms.min(sys.end_ms) as i64 - mic.start_ms.max(sys.start_ms) as i64;
    if overlap <= 0 {
        return false;
    }
    let words = |t: &str| -> Vec<String> {
        t.split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.chars().count() > 2)
            .map(|w| w.to_lowercase())
            .collect()
    };
    let a = words(&mic.text);
    if a.is_empty() {
        return true;
    }
    let b = words(&sys.text);
    let shared = a.iter().filter(|w| b.contains(w)).count();
    shared * 10 >= a.len() * 6
}

pub fn fmt_clock(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// `[mm:ss] Speaker: text` lines, the shape both the summary prompt and the
/// markdown export use.
pub fn to_plain(segments: &[Segment]) -> String {
    let mut out = String::new();
    for s in segments {
        out.push('[');
        out.push_str(&fmt_clock(s.start_ms));
        out.push_str("] ");
        if let Some(sp) = &s.speaker {
            out.push_str(sp);
            out.push_str(": ");
        }
        out.push_str(&s.text);
        out.push('\n');
    }
    out
}

pub fn speakers(segments: &[Segment]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in segments {
        if let Some(sp) = &s.speaker {
            if !out.contains(sp) {
                out.push(sp.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_teams_json_entries() {
        let j = r#"{"entries":[
            {"id":"1","startOffset":"00:00:01.5000000","endOffset":"00:00:03.0000000","speakerDisplayName":"Ana Souza","text":"Bom dia."},
            {"id":"2","startOffset":"00:01:00.0000000","endOffset":"00:01:02.0000000","speakerDisplayName":"","text":"  "},
            {"id":"3","startOffset":"01:00:00.0000000","endOffset":"01:00:01.0000000","speakerDisplayName":"Bruno","text":"Tchau"}
        ]}"#;
        let segs = parse_any(j);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].start_ms, 1500);
        assert_eq!(segs[0].speaker.as_deref(), Some("Ana Souza"));
        assert_eq!(segs[1].start_ms, 3_600_000);
    }

    #[test]
    fn parses_vtt_voice_tags() {
        let vtt = "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\n<v Ana Souza>Olá <b>todos</b></v>\n\n00:00:02.000 --> 00:00:03.000\n<v.loud Bruno>Oi\n\n00:00:03.000 --> 00:00:04.000\nsem falante\n";
        let segs = parse_any(vtt);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].speaker.as_deref(), Some("Ana Souza"));
        assert_eq!(segs[0].text, "Olá todos");
        assert_eq!(segs[1].speaker.as_deref(), Some("Bruno"));
        assert_eq!(segs[2].speaker, None);
    }

    #[test]
    fn merges_turns_of_same_speaker() {
        let seg = |a, b, sp: &str, t: &str| Segment {
            start_ms: a,
            end_ms: b,
            speaker: Some(sp.into()),
            text: t.into(),
        };
        let merged = merge_turns(
            &[
                seg(0, 1000, "A", "um"),
                seg(1200, 2000, "A", "dois"),
                seg(2100, 3000, "B", "três"),
                seg(9000, 9500, "B", "quatro"),
            ],
            1500,
        );
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].text, "um dois");
        assert_eq!(merged[0].end_ms, 2000);
    }

    #[test]
    fn drops_mic_echo_of_system_audio() {
        let s = |a, b, t: &str| Segment {
            start_ms: a,
            end_ms: b,
            speaker: None,
            text: t.into(),
        };
        let merged = merge_tracks(
            vec![
                s(1000, 3000, "we should ship the release friday"),
                s(5000, 6000, "I agree with that plan"),
            ],
            vec![s(900, 3100, "We should ship the release on Friday.")],
        );
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].speaker.as_deref(), Some(SPEAKER_OTHERS));
        assert_eq!(merged[1].speaker.as_deref(), Some(SPEAKER_ME));
        assert_eq!(merged[1].text, "I agree with that plan");
    }

    #[test]
    fn plain_text_has_clock_and_speaker() {
        let p = to_plain(&[Segment {
            start_ms: 65_000,
            end_ms: 66_000,
            speaker: Some("Ana".into()),
            text: "Oi".into(),
        }]);
        assert_eq!(p, "[01:05] Ana: Oi\n");
    }
}
