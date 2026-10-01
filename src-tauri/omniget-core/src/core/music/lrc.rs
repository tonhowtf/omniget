use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LrcWord {
    pub time_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LrcLine {
    pub time_ms: i64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<LrcWord>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Lyrics {
    pub lines: Vec<LrcLine>,
    pub plain: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub offset_ms: i64,
}

impl Lyrics {
    pub fn is_synced(&self) -> bool {
        !self.lines.is_empty()
    }

    pub fn line_at(&self, pos_ms: i64) -> Option<usize> {
        if self.lines.is_empty() || pos_ms < self.lines[0].time_ms {
            return None;
        }
        let idx = self.lines.partition_point(|l| l.time_ms <= pos_ms);
        Some(idx.saturating_sub(1))
    }
}

fn parse_stamp(s: &str) -> Option<i64> {
    let s = s.trim();
    let (main, frac) = match s.rfind(['.', ',']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let parts: Vec<&str> = main.split(':').collect();
    let (h, m, sec, frac) = match (parts.len(), frac) {
        (2, f) => (0, parts[0], parts[1], f),
        (3, None) if parts[2].len() == 2 && parts[0].len() <= 3 => {
            (0, parts[0], parts[1], Some(parts[2]))
        }
        (3, f) => (parts[0].parse::<i64>().ok()?, parts[1], parts[2], f),
        _ => return None,
    };
    let m: i64 = m.trim().parse().ok()?;
    let sec: i64 = sec.trim().parse().ok()?;
    if sec >= 60 || m < 0 {
        return None;
    }
    let frac_ms = match frac {
        None | Some("") => 0,
        Some(f) => {
            if !f.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let digits: String = f.chars().take(3).collect();
            let v: i64 = digits.parse().ok()?;
            match digits.len() {
                1 => v * 100,
                2 => v * 10,
                _ => v,
            }
        }
    };
    Some(((h * 60 + m) * 60 + sec) * 1000 + frac_ms)
}

fn parse_words(text: &str) -> (String, Vec<LrcWord>) {
    if !text.contains('<') {
        return (text.trim().to_string(), Vec::new());
    }
    let mut words = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    let mut current: Option<i64> = None;
    loop {
        match rest.find('<') {
            Some(open) => {
                let before = &rest[..open];
                if !before.is_empty() {
                    plain.push_str(before);
                    if let Some(t) = current {
                        if !before.trim().is_empty() {
                            words.push(LrcWord {
                                time_ms: t,
                                text: before.to_string(),
                            });
                        }
                    }
                }
                let Some(close) = rest[open..].find('>') else {
                    plain.push_str(&rest[open..]);
                    break;
                };
                let inner = &rest[open + 1..open + close];
                match parse_stamp(inner) {
                    Some(t) => current = Some(t),
                    None => plain.push_str(&rest[open..open + close + 1]),
                }
                rest = &rest[open + close + 1..];
            }
            None => {
                if !rest.is_empty() {
                    plain.push_str(rest);
                    if let Some(t) = current {
                        if !rest.trim().is_empty() {
                            words.push(LrcWord {
                                time_ms: t,
                                text: rest.to_string(),
                            });
                        }
                    }
                }
                break;
            }
        }
    }
    if words.is_empty() && current.is_none() {
        return (plain.trim().to_string(), Vec::new());
    }
    (
        plain.split_whitespace().collect::<Vec<_>>().join(" "),
        words,
    )
}

pub fn parse(input: &str) -> Lyrics {
    let mut out = Lyrics::default();
    let mut plain_lines = Vec::new();
    let mut any_stamp = false;

    for raw in input.lines() {
        let line = raw.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            plain_lines.push(String::new());
            continue;
        }
        let mut stamps = Vec::new();
        let mut rest = line;
        let mut is_tag = false;
        while rest.starts_with('[') {
            let Some(close) = rest.find(']') else { break };
            let inner = &rest[1..close];
            if let Some(t) = parse_stamp(inner) {
                stamps.push(t);
            } else if let Some((k, v)) = inner.split_once(':') {
                let v = v.trim().to_string();
                match k.trim().to_ascii_lowercase().as_str() {
                    "ti" => out.title = Some(v),
                    "ar" => out.artist = Some(v),
                    "al" => out.album = Some(v),
                    "offset" => out.offset_ms = v.trim_start_matches('+').parse().unwrap_or(0),
                    _ => {}
                }
                is_tag = true;
            } else {
                break;
            }
            rest = &rest[close + 1..];
        }
        if stamps.is_empty() {
            if !is_tag {
                plain_lines.push(line.to_string());
            }
            continue;
        }
        any_stamp = true;
        for t in stamps {
            let (text, words) = parse_words(rest);
            out.lines.push(LrcLine {
                time_ms: t,
                text,
                words,
            });
        }
    }

    if any_stamp {
        let off = out.offset_ms;
        for l in &mut out.lines {
            l.time_ms = (l.time_ms - off).max(0);
            for w in &mut l.words {
                w.time_ms = (w.time_ms - off).max(0);
            }
        }
        out.lines.sort_by_key(|l| l.time_ms);
    } else {
        let text = plain_lines.join("\n").trim().to_string();
        if !text.is_empty() {
            out.plain = Some(text);
        }
    }
    out
}

fn fmt_stamp(ms: i64) -> String {
    let ms = ms.max(0);
    let cs = (ms % 1000) / 10;
    let s = (ms / 1000) % 60;
    let m = ms / 60_000;
    format!("{m:02}:{s:02}.{cs:02}")
}

pub fn serialize(l: &Lyrics) -> String {
    let mut out = String::new();
    if let Some(v) = &l.title {
        out.push_str(&format!("[ti:{v}]\n"));
    }
    if let Some(v) = &l.artist {
        out.push_str(&format!("[ar:{v}]\n"));
    }
    if let Some(v) = &l.album {
        out.push_str(&format!("[al:{v}]\n"));
    }
    if l.lines.is_empty() {
        if let Some(p) = &l.plain {
            out.push_str(p);
            out.push('\n');
        }
        return out;
    }
    for line in &l.lines {
        out.push('[');
        out.push_str(&fmt_stamp(line.time_ms));
        out.push(']');
        if line.words.is_empty() {
            out.push_str(&line.text);
        } else {
            for w in &line.words {
                out.push('<');
                out.push_str(&fmt_stamp(w.time_ms));
                out.push('>');
                out.push_str(&w.text);
            }
        }
        out.push('\n');
    }
    out
}

pub fn from_subtitles(input: &str) -> Lyrics {
    let mut out = Lyrics::default();
    let mut pending: Option<i64> = None;
    let mut buf: Vec<String> = Vec::new();
    let flush = |out: &mut Lyrics, pending: &mut Option<i64>, buf: &mut Vec<String>| {
        if let Some(t) = pending.take() {
            let text = buf.join(" ");
            let text = strip_tags(&text);
            let text = text.trim();
            if !text.is_empty() && out.lines.last().map(|l| l.text.as_str()) != Some(text) {
                out.lines.push(LrcLine {
                    time_ms: t,
                    text: text.to_string(),
                    words: Vec::new(),
                });
            }
        }
        buf.clear();
    };
    for raw in input.lines() {
        let line = raw.trim();
        if let Some((start, _)) = line.split_once("-->") {
            flush(&mut out, &mut pending, &mut buf);
            let start = start.trim().replace(',', ".");
            pending = parse_stamp(&start);
            continue;
        }
        if line.is_empty() {
            flush(&mut out, &mut pending, &mut buf);
            continue;
        }
        if pending.is_some() {
            buf.push(line.to_string());
        }
    }
    flush(&mut out, &mut pending, &mut buf);
    out.lines.sort_by_key(|l| l.time_ms);
    out
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_and_multi_stamp_lines() {
        let l = parse("[ti:Song]\n[ar:Band]\n[00:01.50]one\n[00:10.00][00:20.00]chorus\n");
        assert_eq!(l.title.as_deref(), Some("Song"));
        assert_eq!(l.artist.as_deref(), Some("Band"));
        assert_eq!(l.lines.len(), 3);
        assert_eq!(l.lines[0].time_ms, 1500);
        assert_eq!(l.lines[1].text, "chorus");
        assert_eq!(l.lines[2].time_ms, 20_000);
    }

    #[test]
    fn applies_offset_and_fraction_widths() {
        let l = parse("[offset:+500]\n[00:02.5]a\n[00:03.123]b\n");
        assert_eq!(l.lines[0].time_ms, 2000);
        assert_eq!(l.lines[1].time_ms, 2623);
    }

    #[test]
    fn keeps_enhanced_word_stamps() {
        let l = parse("[00:01.00]<00:01.00>Hel<00:01.40>lo <00:02.00>world\n");
        assert_eq!(l.lines[0].words.len(), 3);
        assert_eq!(l.lines[0].words[2].time_ms, 2000);
        assert_eq!(l.lines[0].text, "Hello world");
    }

    #[test]
    fn unsynced_text_becomes_plain() {
        let l = parse("just words\nmore words\n");
        assert!(!l.is_synced());
        assert_eq!(l.plain.as_deref(), Some("just words\nmore words"));
    }

    #[test]
    fn line_at_finds_current_line() {
        let l = parse("[00:01.00]a\n[00:05.00]b\n");
        assert_eq!(l.line_at(0), None);
        assert_eq!(l.line_at(1000), Some(0));
        assert_eq!(l.line_at(4999), Some(0));
        assert_eq!(l.line_at(9000), Some(1));
    }

    #[test]
    fn roundtrips() {
        let l = parse("[00:01.50]one\n[01:02.03]two\n");
        let again = parse(&serialize(&l));
        assert_eq!(l.lines, again.lines);
    }

    #[test]
    fn converts_vtt() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\n<c>Hello</c> there\n\n00:00:04.000 --> 00:00:05.000\nagain\n";
        let l = from_subtitles(vtt);
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[0].text, "Hello there");
        assert_eq!(l.lines[1].time_ms, 4000);
    }

    #[test]
    fn converts_srt() {
        let srt = "1\n00:00:01,500 --> 00:00:03,000\nLine one\n\n2\n00:00:04,000 --> 00:00:05,000\nLine two\n";
        let l = from_subtitles(srt);
        assert_eq!(l.lines[0].time_ms, 1500);
        assert_eq!(l.lines[1].text, "Line two");
    }
}
