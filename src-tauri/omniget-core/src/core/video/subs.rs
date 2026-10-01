use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return encoding_rs::UTF_16LE.decode(bytes).0.into_owned();
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return encoding_rs::UTF_16BE.decode(bytes).0.into_owned();
    }
    if bytes.len() >= 4 {
        if bytes[1] == 0 && bytes[3] == 0 && bytes[0] != 0 {
            return encoding_rs::UTF_16LE
                .decode_without_bom_handling(bytes)
                .0
                .into_owned();
        }
        if bytes[0] == 0 && bytes[2] == 0 && bytes[1] != 0 {
            return encoding_rs::UTF_16BE
                .decode_without_bom_handling(bytes)
                .0
                .into_owned();
        }
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252
            .decode_without_bom_handling(bytes)
            .0
            .into_owned(),
    }
}

fn normalize_newlines(text: &str) -> String {
    text.trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

pub fn parse_timestamp(s: &str) -> Option<u64> {
    let s = s.trim();
    let (main, frac) = match s.find([',', '.']) {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    };
    let parts: Vec<&str> = main.split(':').map(str::trim).collect();
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let mut secs: u64 = 0;
    for p in &parts {
        if !p.chars().all(|c| c.is_ascii_digit()) || p.len() > 6 {
            return None;
        }
        secs = secs * 60 + p.parse::<u64>().ok()?;
    }
    let frac: String = frac.chars().take_while(|c| c.is_ascii_digit()).collect();
    let ms = match frac.len() {
        0 => 0,
        1 => frac.parse::<u64>().ok()? * 100,
        2 => frac.parse::<u64>().ok()? * 10,
        _ => frac[..3].parse::<u64>().ok()?,
    };
    Some(secs * 1000 + ms)
}

fn parse_timing(line: &str) -> Option<(u64, u64)> {
    let (a, b) = line.split_once("-->")?;
    let start = parse_timestamp(a.split_whitespace().last()?)?;
    let end = parse_timestamp(b.split_whitespace().next()?)?;
    Some((start, end.max(start)))
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lrm;", "")
        .replace("&rlm;", "")
        .replace("&amp;", "&")
}

fn clean_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                let rest: String = chars.clone().take(64).collect();
                if let Some(end) = rest.find('>') {
                    let inner = &rest[..end];
                    if !inner.is_empty() && !inner.contains('<') && !inner.starts_with(' ') {
                        for _ in 0..=end {
                            chars.next();
                        }
                        continue;
                    }
                }
                out.push(c);
            }
            '{' if chars.peek() == Some(&'\\') => {
                for n in chars.by_ref() {
                    if n == '}' {
                        break;
                    }
                }
            }
            _ => out.push(c),
        }
    }
    decode_entities(&out)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_timed(text: &str, vtt: bool) -> Vec<Cue> {
    let text = normalize_newlines(text);
    let mut cues = Vec::new();
    let mut current: Option<(u64, u64, Vec<&str>)> = None;
    let mut skipping_block = false;
    let mut in_header = vtt;
    let flush = |cur: &mut Option<(u64, u64, Vec<&str>)>, cues: &mut Vec<Cue>| {
        if let Some((s, e, lines)) = cur.take() {
            let t = clean_markup(&lines.join("\n"));
            if !t.is_empty() {
                cues.push(Cue {
                    start_ms: s,
                    end_ms: e,
                    text: t,
                });
            }
        }
    };
    for line in text.split('\n') {
        let trimmed = line.trim();
        if in_header {
            if trimmed.is_empty() {
                in_header = false;
            }
            if !trimmed.contains("-->") {
                continue;
            }
            in_header = false;
        }
        if trimmed.contains("-->") {
            if let Some(t) = parse_timing(trimmed) {
                if let Some((_, _, lines)) = current.as_mut() {
                    if lines
                        .last()
                        .is_some_and(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_digit()))
                    {
                        lines.pop();
                    }
                }
                flush(&mut current, &mut cues);
                skipping_block = false;
                current = Some((t.0, t.1, Vec::new()));
                continue;
            }
        }
        if trimmed.is_empty() {
            flush(&mut current, &mut cues);
            skipping_block = false;
            continue;
        }
        if skipping_block {
            continue;
        }
        match current.as_mut() {
            Some((_, _, lines)) => lines.push(trimmed),
            None => {
                if vtt
                    && (trimmed.starts_with("NOTE")
                        || trimmed.starts_with("STYLE")
                        || trimmed.starts_with("REGION"))
                {
                    skipping_block = true;
                }
            }
        }
    }
    flush(&mut current, &mut cues);
    cues.sort_by_key(|c| (c.start_ms, c.end_ms));
    cues
}

pub fn parse_srt(text: &str) -> Vec<Cue> {
    parse_timed(text, false)
}

pub fn parse_vtt(text: &str) -> Vec<Cue> {
    parse_timed(text, true)
}

fn ass_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut drawing = false;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            let mut block = String::new();
            for n in chars.by_ref() {
                if n == '}' {
                    break;
                }
                block.push(n);
            }
            if let Some(i) = block.rfind("\\p") {
                let digit = block[i + 2..].chars().next();
                if let Some(d) = digit.filter(|d| d.is_ascii_digit()) {
                    drawing = d != '0';
                }
            }
            continue;
        }
        if drawing {
            continue;
        }
        if c == '\\' {
            match chars.peek() {
                Some('N') | Some('n') => {
                    chars.next();
                    out.push('\n');
                    continue;
                }
                Some('h') => {
                    chars.next();
                    out.push(' ');
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn parse_ass(text: &str) -> Vec<Cue> {
    let text = normalize_newlines(text);
    let default_format: Vec<String> = [
        "layer", "start", "end", "style", "name", "marginl", "marginr", "marginv", "effect", "text",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let mut format = default_format.clone();
    let mut in_events = false;
    let mut cues: Vec<Cue> = Vec::new();
    for line in text.split('\n') {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            in_events = t.eq_ignore_ascii_case("[events]");
            continue;
        }
        if !in_events {
            continue;
        }
        let Some((key, value)) = t.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        if key == "format" {
            let f: Vec<String> = value
                .split(',')
                .map(|s| s.trim().to_ascii_lowercase())
                .collect();
            format = if f.iter().any(|x| x == "text") && f.iter().any(|x| x == "start") {
                f
            } else {
                default_format.clone()
            };
            continue;
        }
        if key != "dialogue" {
            continue;
        }
        let fields: Vec<&str> = value.splitn(format.len(), ',').collect();
        if fields.len() < format.len() {
            continue;
        }
        let idx = |name: &str| format.iter().position(|f| f == name);
        let (Some(si), Some(ei), Some(ti)) = (idx("start"), idx("end"), idx("text")) else {
            continue;
        };
        let (Some(start), Some(end)) = (parse_timestamp(fields[si]), parse_timestamp(fields[ei]))
        else {
            continue;
        };
        let body = ass_text(fields[ti]);
        if body.is_empty() {
            continue;
        }
        cues.push(Cue {
            start_ms: start,
            end_ms: end.max(start),
            text: body,
        });
    }
    cues.sort_by_key(|c| (c.start_ms, c.end_ms));
    cues.dedup();
    cues
}

pub fn parse_auto(bytes: &[u8]) -> Vec<Cue> {
    let text = decode_text(bytes);
    let head = text.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("WEBVTT") {
        return parse_vtt(&text);
    }
    let lower_head: String = head.chars().take(4096).collect::<String>().to_lowercase();
    if lower_head.starts_with("[script info]")
        || lower_head.contains("[events]")
        || lower_head.contains("\ndialogue:")
        || lower_head.contains("[v4+ styles]")
        || lower_head.contains("[v4 styles]")
    {
        let cues = parse_ass(&text);
        if !cues.is_empty() {
            return cues;
        }
    }
    parse_srt(&text)
}

fn vtt_time(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        ms % 1000
    )
}

fn shift(ms: u64, offset_ms: i64) -> i64 {
    (ms as i64).saturating_add(offset_ms)
}

pub fn to_vtt(cues: &[Cue], offset_ms: i64) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for c in cues {
        let end = shift(c.end_ms, offset_ms);
        if end <= 0 {
            continue;
        }
        let start = shift(c.start_ms, offset_ms).max(0) as u64;
        let end = (end as u64).max(start);
        let text: Vec<String> = c
            .text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| {
                l.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            })
            .collect();
        if text.is_empty() {
            continue;
        }
        out.push_str(&vtt_time(start));
        out.push_str(" --> ");
        out.push_str(&vtt_time(end));
        out.push('\n');
        out.push_str(&text.join("\n"));
        out.push_str("\n\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(s: u64, e: u64, t: &str) -> Cue {
        Cue {
            start_ms: s,
            end_ms: e,
            text: t.into(),
        }
    }

    #[test]
    fn srt_with_bom_and_crlf() {
        let src = "\u{feff}1\r\n00:00:01,000 --> 00:00:02,500\r\n<i>Olá</i> mundo\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000\r\n{\\an8}Linha 1\r\nLinha 2\r\n";
        let cues = parse_srt(src);
        assert_eq!(
            cues,
            vec![
                cue(1000, 2500, "Olá mundo"),
                cue(3000, 4000, "Linha 1\nLinha 2")
            ]
        );
        assert_eq!(parse_auto(src.as_bytes()), cues);
    }

    #[test]
    fn srt_tolerant() {
        let src = "1\n00:01.000 --> 00:02.000\nA\n2\n00:00:03.5 --> 00:00:04\nB\n\n\n3\n99:99 --> junk\nC";
        let cues = parse_srt(src);
        assert_eq!(cues, vec![cue(1000, 2000, "A"), cue(3500, 4000, "B")]);
        assert!(parse_srt("").is_empty());
        assert!(parse_srt("garbage\n\n-->\n").is_empty());
    }

    #[test]
    fn vtt_basic() {
        let src = "WEBVTT - title\nKind: captions\n\nNOTE a comment\nwith --> arrow? no\n\nSTYLE\n::cue { color: red }\n\nintro\n00:00:00.500 --> 00:00:01.000 align:start position:10%\n<v Bob>Hello &amp; <c.yellow>bye</c>\n\n00:01:00.000 --> 00:01:02.000\n<00:01:00.500>word\n";
        let cues = parse_vtt(src);
        assert_eq!(
            cues,
            vec![cue(500, 1000, "Hello & bye"), cue(60000, 62000, "word")]
        );
        assert_eq!(parse_auto(src.as_bytes()), cues);
    }

    #[test]
    fn ass_tags_and_newlines() {
        let src = "[Script Info]\nTitle: x\n\n[V4+ Styles]\nFormat: Name, Fontname\nStyle: Default,Arial\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,{\\i1}Oi,{\\b1} tudo\\Nbem?{\\i0}\nComment: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,ignored\nDialogue: 0,0:00:05.00,0:00:06.00,Default,,0,0,0,,{\\p1}m 0 0 l 100 0{\\p0}\nDialogue: 0,0:00:07.10,0:00:08.00,Default,,0,0,0,,a\\hb\n";
        let cues = parse_ass(src);
        assert_eq!(
            cues,
            vec![cue(1000, 2500, "Oi, tudo\nbem?"), cue(7100, 8000, "a b")]
        );
        assert_eq!(parse_auto(src.as_bytes()), cues);
    }

    #[test]
    fn ass_custom_format_order() {
        let src = "[Events]\nFormat: Start, End, Text\nDialogue: 0:00:01.00,0:00:02.00,x, y\n";
        assert_eq!(parse_ass(src), vec![cue(1000, 2000, "x, y")]);
    }

    #[test]
    fn encodings() {
        let bytes = b"1\n00:00:01,000 --> 00:00:02,000\na\xe7\xe3o\n";
        assert_eq!(parse_auto(bytes)[0].text, "ação");
        let s = "1\n00:00:01,000 --> 00:00:02,000\nção\n";
        let mut b = vec![0xFF, 0xFE];
        for u in s.encode_utf16() {
            b.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(parse_auto(&b)[0].text, "ção");
        assert!(parse_auto(&[]).is_empty());
        assert!(parse_auto(&[0xff, 0x00, 0x13]).is_empty());
    }

    #[test]
    fn timestamps() {
        assert_eq!(parse_timestamp("01:02:03,456"), Some(3_723_456));
        assert_eq!(parse_timestamp("0:00:01.5"), Some(1500));
        assert_eq!(parse_timestamp("0:00:01.25"), Some(1250));
        assert_eq!(parse_timestamp("02:03"), Some(123_000));
        assert_eq!(parse_timestamp("a:b"), None);
        assert_eq!(parse_timestamp(""), None);
    }

    #[test]
    fn vtt_output_with_offset() {
        let cues = vec![
            cue(500, 1500, "gone"),
            cue(2000, 3000, "a < b & c"),
            cue(3_600_000, 3_601_000, "x\n\ny"),
        ];
        let out = to_vtt(&cues, -1500);
        assert_eq!(
            out,
            "WEBVTT\n\n00:00:00.500 --> 00:00:01.500\na &lt; b &amp; c\n\n00:59:58.500 --> 00:59:59.500\nx\ny\n\n"
        );
        let out = to_vtt(&cues, 250);
        assert!(out.contains("00:00:00.750 --> 00:00:01.750\ngone"));
        assert_eq!(parse_vtt(&to_vtt(&cues, 0)).len(), 3);
    }
}
