struct SubBlock {
    timing: String,
    text: String,
}

fn parse_srt(content: &str) -> Vec<SubBlock> {
    let normalized = content.replace('\r', "");
    let mut blocks = Vec::new();
    for part in normalized.split("\n\n") {
        let lines: Vec<&str> = part.trim().split('\n').collect();
        if lines.len() >= 3 {
            blocks.push(SubBlock {
                timing: lines[1].to_string(),
                text: lines[2..].join("\n"),
            });
        }
    }
    blocks
}

fn parse_vtt(content: &str) -> Vec<SubBlock> {
    let normalized = content.replace('\r', "");
    let mut blocks = Vec::new();
    for part in normalized.split("\n\n") {
        let lines: Vec<&str> = part.trim().split('\n').collect();
        if lines.is_empty() {
            continue;
        }
        if let Some(idx) = lines.iter().position(|l| l.contains("-->")) {
            if idx + 1 < lines.len() {
                blocks.push(SubBlock {
                    timing: lines[idx].to_string(),
                    text: lines[idx + 1..].join("\n"),
                });
            }
        }
    }
    blocks
}

fn merge(primary: &[SubBlock], secondary: &[SubBlock], header: &[&str], numbered: bool) -> String {
    let max_len = primary.len().max(secondary.len());
    let mut out: Vec<String> = header.iter().map(|s| s.to_string()).collect();
    for i in 0..max_len {
        if numbered {
            out.push((i + 1).to_string());
        }
        match (primary.get(i), secondary.get(i)) {
            (Some(pb), sb) => {
                out.push(pb.timing.clone());
                out.push(pb.text.clone());
                if let Some(sb) = sb {
                    out.push(sb.text.clone());
                }
            }
            (None, Some(sb)) => {
                out.push(sb.timing.clone());
                out.push(sb.text.clone());
            }
            (None, None) => {}
        }
        out.push(String::new());
    }
    out.join("\r\n")
}

pub fn merge_bilingual_srt(primary: &str, secondary: &str) -> String {
    merge(&parse_srt(primary), &parse_srt(secondary), &[], true)
}

pub fn merge_bilingual_vtt(primary: &str, secondary: &str) -> String {
    merge(
        &parse_vtt(primary),
        &parse_vtt(secondary),
        &["WEBVTT", ""],
        false,
    )
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

fn parse_ts(s: &str) -> Option<u64> {
    let s = s.trim();
    let (hms, ms_part) = if let Some((a, b)) = s.split_once(',') {
        (a, b)
    } else if let Some((a, b)) = s.rsplit_once('.') {
        (a, b)
    } else {
        (s, "0")
    };
    let ms: u64 = ms_part
        .chars()
        .take(3)
        .collect::<String>()
        .parse()
        .ok()
        .map(|v: u64| v * 10u64.pow(3u32.saturating_sub(ms_part.len().min(3) as u32)))
        .unwrap_or(0);
    let parts: Vec<&str> = hms.split(':').collect();
    let (h, m, sec): (u64, u64, u64) = match parts.as_slice() {
        [h, m, s] => (
            h.trim().parse::<u64>().ok()?,
            m.trim().parse::<u64>().ok()?,
            s.trim().parse::<u64>().ok()?,
        ),
        [m, s] => (
            0,
            m.trim().parse::<u64>().ok()?,
            s.trim().parse::<u64>().ok()?,
        ),
        _ => return None,
    };
    Some(((h * 3600 + m * 60 + sec) * 1000) + ms)
}

fn fmt_ts(ms: u64) -> String {
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1000;
    let milli = ms % 1000;
    format!("{:02}:{:02}:{:02},{:03}", h, m, s, milli)
}

fn parse_ass_ts(s: &str) -> Option<u64> {
    let s = s.trim();
    let (hms, cc) = s.rsplit_once('.')?;
    let centis: u64 = cc.chars().take(2).collect::<String>().parse().ok()?;
    let parts: Vec<&str> = hms.split(':').collect();
    let (h, m, sec): (u64, u64, u64) = match parts.as_slice() {
        [h, m, s] => (
            h.trim().parse().ok()?,
            m.trim().parse().ok()?,
            s.trim().parse().ok()?,
        ),
        [m, s] => (0, m.trim().parse().ok()?, s.trim().parse().ok()?),
        _ => return None,
    };
    Some(((h * 3600 + m * 60 + sec) * 1000) + centis * 10)
}

fn fmt_ass_ts(ms: u64) -> String {
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1000;
    let cs = (ms % 1000) / 10;
    format!("{}:{:02}:{:02}.{:02}", h, m, s, cs)
}

fn strip_ass_overrides(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut depth = 0u32;
    for ch in raw.chars() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.replace("\\N", "\n")
        .replace("\\n", "\n")
        .trim()
        .to_string()
}

pub fn parse_cues_ass(content: &str) -> Vec<Cue> {
    let normalized = content.replace('\r', "");
    let mut cues = Vec::new();
    let mut in_events = false;
    let mut start_col = 1usize;
    let mut end_col = 2usize;
    let mut text_col = 9usize;

    for line in normalized.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_events = trimmed.eq_ignore_ascii_case("[Events]");
            continue;
        }
        if !in_events {
            continue;
        }
        if let Some(fmt) = trimmed.strip_prefix("Format:") {
            for (i, name) in fmt.split(',').map(|s| s.trim().to_lowercase()).enumerate() {
                match name.as_str() {
                    "start" => start_col = i,
                    "end" => end_col = i,
                    "text" => text_col = i,
                    _ => {}
                }
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Dialogue:") {
            let max_split = text_col.max(start_col).max(end_col);
            let fields: Vec<&str> = rest.splitn(max_split + 1, ',').collect();
            let (Some(s), Some(e), Some(t)) = (
                fields.get(start_col).and_then(|v| parse_ass_ts(v)),
                fields.get(end_col).and_then(|v| parse_ass_ts(v)),
                fields.get(text_col),
            ) else {
                continue;
            };
            let text = strip_ass_overrides(t);
            if !text.is_empty() {
                cues.push(Cue {
                    start_ms: s,
                    end_ms: e,
                    text,
                });
            }
        }
    }
    cues
}

pub fn cues_to_ass(cues: &[Cue]) -> String {
    let mut out = String::from(
        "[Script Info]\nScriptType: v4.00+\nWrapStyle: 0\nScaledBorderAndShadow: yes\n\n\
[V4+ Styles]\n\
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Default,Arial,48,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,2,10,10,10,1\n\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );
    for c in cues {
        let text = c.text.replace('\n', "\\N");
        out.push_str(&format!(
            "Dialogue: 0,{},{},Default,,0,0,0,,{}\n",
            fmt_ass_ts(c.start_ms),
            fmt_ass_ts(c.end_ms),
            text
        ));
    }
    out
}

pub fn parse_cues(content: &str) -> Vec<Cue> {
    if content.contains("[Events]") || content.contains("Dialogue:") {
        return parse_cues_ass(content);
    }
    let normalized = content.replace('\r', "");
    let mut cues = Vec::new();
    for part in normalized.split("\n\n") {
        let lines: Vec<&str> = part.trim().split('\n').collect();
        let Some(idx) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let timing = lines[idx];
        let Some((a, b)) = timing.split_once("-->") else {
            continue;
        };
        let (Some(start_ms), Some(end_ms)) = (
            parse_ts(a),
            parse_ts(b.split_whitespace().next().unwrap_or(b)),
        ) else {
            continue;
        };
        let text = lines[idx + 1..].join("\n").trim().to_string();
        if text.is_empty() {
            continue;
        }
        cues.push(Cue {
            start_ms,
            end_ms,
            text,
        });
    }
    cues
}

pub fn cues_to_srt(cues: &[Cue]) -> String {
    let mut out = String::new();
    for (i, c) in cues.iter().enumerate() {
        out.push_str(&(i + 1).to_string());
        out.push('\n');
        out.push_str(&fmt_ts(c.start_ms));
        out.push_str(" --> ");
        out.push_str(&fmt_ts(c.end_ms));
        out.push('\n');
        out.push_str(&c.text);
        out.push_str("\n\n");
    }
    out
}

pub fn cues_to_vtt(cues: &[Cue]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for c in cues {
        out.push_str(&fmt_ts(c.start_ms).replace(',', "."));
        out.push_str(" --> ");
        out.push_str(&fmt_ts(c.end_ms).replace(',', "."));
        out.push('\n');
        out.push_str(&c.text);
        out.push_str("\n\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_blocks_parsed_index_aligned() {
        let p =
            "1\n00:00:01,000 --> 00:00:02,000\nHello\n\n2\n00:00:03,000 --> 00:00:04,000\nWorld";
        let s = "1\n00:00:01,000 --> 00:00:02,000\nOlá\n\n2\n00:00:03,000 --> 00:00:04,000\nMundo";
        let merged = merge_bilingual_srt(p, s);
        assert!(merged.contains("Hello\r\nOlá"));
        assert!(merged.contains("World\r\nMundo"));
        assert!(merged.starts_with("1\r\n"));
    }

    #[test]
    fn vtt_has_header_and_no_index() {
        let p = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHi";
        let s = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nOi";
        let merged = merge_bilingual_vtt(p, s);
        assert!(merged.starts_with("WEBVTT\r\n"));
        assert!(merged.contains("Hi\r\nOi"));
        assert!(!merged.contains("\r\n1\r\n"));
    }

    #[test]
    fn uneven_lengths_fall_back_to_secondary_timing() {
        let p = "1\n00:00:01,000 --> 00:00:02,000\nOne";
        let s = "1\n00:00:01,000 --> 00:00:02,000\nUm\n\n2\n00:00:05,000 --> 00:00:06,000\nDois";
        let merged = merge_bilingual_srt(p, s);
        assert!(merged.contains("One\r\nUm"));
        assert!(merged.contains("00:00:05,000 --> 00:00:06,000\r\nDois"));
    }

    #[test]
    fn empty_inputs_are_safe() {
        assert_eq!(merge_bilingual_srt("", ""), "");
        assert_eq!(merge_bilingual_vtt("", ""), "WEBVTT\r\n");
    }

    #[test]
    fn parse_cues_srt_roundtrip() {
        let srt =
            "1\n00:00:01,500 --> 00:00:02,250\nHello\n\n2\n00:01:00,000 --> 00:01:02,000\nWorld";
        let cues = parse_cues(srt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 1500);
        assert_eq!(cues[0].end_ms, 2250);
        assert_eq!(cues[1].start_ms, 60000);
        assert_eq!(cues[0].text, "Hello");
        let back = cues_to_srt(&cues);
        let again = parse_cues(&back);
        assert_eq!(again[1].start_ms, 60000);
        assert_eq!(again[1].text, "World");
    }

    #[test]
    fn parse_cues_vtt() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHi there";
        let cues = parse_cues(vtt);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start_ms, 1000);
        assert_eq!(cues[0].text, "Hi there");
        assert!(cues_to_vtt(&cues).starts_with("WEBVTT"));
    }

    #[test]
    fn ass_roundtrip_and_autodetect() {
        let cues = vec![
            Cue {
                start_ms: 1500,
                end_ms: 2250,
                text: "Hello".to_string(),
            },
            Cue {
                start_ms: 60000,
                end_ms: 62000,
                text: "two\nlines".to_string(),
            },
        ];
        let ass = cues_to_ass(&cues);
        assert!(ass.contains("[Events]"));
        assert!(ass.contains("Dialogue: 0,0:00:01.50,0:00:02.25,Default,,0,0,0,,Hello"));
        let back = parse_cues(&ass);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].start_ms, 1500);
        assert_eq!(back[0].end_ms, 2250);
        assert_eq!(back[1].start_ms, 60000);
        assert_eq!(back[1].text, "two\nlines");
    }

    #[test]
    fn ass_strips_override_tags_and_respects_format_columns() {
        let ass = "[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:03.00,0:00:04.50,Default,,0,0,0,,{\\b1}Bold{\\b0} text";
        let cues = parse_cues_ass(ass);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start_ms, 3000);
        assert_eq!(cues[0].end_ms, 4500);
        assert_eq!(cues[0].text, "Bold text");
    }
}
