use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistEntry {
    pub location: String,
    pub title: Option<String>,
    pub duration_s: Option<f64>,
    pub logo: Option<String>,
    pub group: Option<String>,
}

impl PlaylistEntry {
    pub fn is_url(&self) -> bool {
        is_url(&self.location)
    }
}

fn is_url(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://") || l.starts_with("rtmp://")
}

fn resolve(loc: &str, base: Option<&Path>) -> String {
    let loc = loc.trim();
    if is_url(loc) {
        return loc.to_string();
    }
    let loc = loc.strip_prefix("file://").unwrap_or(loc);
    let decoded = urlencoding::decode(loc)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| loc.to_string());
    let normalized = if cfg!(windows) {
        decoded
    } else {
        decoded.replace('\\', "/")
    };
    let p = PathBuf::from(&normalized);
    if p.is_absolute() {
        return normalized;
    }
    match base {
        Some(b) => b.join(p).to_string_lossy().into_owned(),
        None => normalized,
    }
}

fn attr(s: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let start = s.find(&needle)? + needle.len();
    let end = s[start..].find('"')? + start;
    let v = s[start..end].trim();
    (!v.is_empty()).then(|| v.to_string())
}

fn first_unquoted_comma(s: &str) -> Option<usize> {
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => return Some(i),
            _ => {}
        }
    }
    None
}

pub fn parse_m3u(text: &str, base: Option<&Path>) -> Vec<PlaylistEntry> {
    let mut out = Vec::new();
    let mut title = None;
    let mut dur = None;
    let mut logo = None;
    let mut group = None;
    for raw in text.lines() {
        let line = raw.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            let (head, t) = match first_unquoted_comma(info) {
                Some(i) => (&info[..i], &info[i + 1..]),
                None => (info, ""),
            };
            let num: String = head
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                .collect();
            dur = num.parse::<f64>().ok().filter(|d| *d >= 0.0);
            logo = attr(head, "tvg-logo");
            group = attr(head, "group-title");
            let t = t.trim();
            title = (!t.is_empty()).then(|| t.to_string());
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        out.push(PlaylistEntry {
            location: resolve(line, base),
            title: title.take(),
            duration_s: dur.take(),
            logo: logo.take(),
            group: group.take(),
        });
    }
    out
}

pub fn parse_pls(text: &str, base: Option<&Path>) -> Vec<PlaylistEntry> {
    use std::collections::BTreeMap;
    let mut files: BTreeMap<u32, PlaylistEntry> = BTreeMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim().to_ascii_lowercase();
        let idx_of = |prefix: &str| k.strip_prefix(prefix).and_then(|n| n.parse::<u32>().ok());
        let blank = || PlaylistEntry {
            location: String::new(),
            title: None,
            duration_s: None,
            logo: None,
            group: None,
        };
        if let Some(n) = idx_of("file") {
            files.entry(n).or_insert_with(blank).location = resolve(v, base);
        } else if let Some(n) = idx_of("title") {
            files.entry(n).or_insert_with(blank).title = Some(v.trim().to_string());
        } else if let Some(n) = idx_of("length") {
            files.entry(n).or_insert_with(blank).duration_s =
                v.trim().parse::<f64>().ok().filter(|d| *d >= 0.0);
        }
    }
    files
        .into_values()
        .filter(|e| !e.location.is_empty())
        .collect()
}

pub fn parse_any(text: &str, name_hint: &str, base: Option<&Path>) -> Vec<PlaylistEntry> {
    let lower = name_hint.to_ascii_lowercase();
    if lower.ends_with(".pls")
        || text
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("[playlist]")
    {
        parse_pls(text, base)
    } else {
        parse_m3u(text, base)
    }
}

pub fn write_m3u(entries: &[PlaylistEntry], base: Option<&Path>) -> String {
    let mut out = String::from("#EXTM3U\n");
    for e in entries {
        let dur = e.duration_s.map(|d| d.round() as i64).unwrap_or(-1);
        out.push_str(&format!(
            "#EXTINF:{dur},{}\n",
            e.title.clone().unwrap_or_default()
        ));
        let loc = match base {
            Some(b) if !e.is_url() => Path::new(&e.location)
                .strip_prefix(b)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| e.location.clone()),
            _ => e.location.clone(),
        };
        out.push_str(&loc);
        out.push('\n');
    }
    out
}

pub fn absolutize_hls(text: &str, base: &url::Url) -> String {
    let abs = |u: &str| -> String {
        base.join(u.trim())
            .map(|x| x.to_string())
            .unwrap_or_else(|_| u.to_string())
    };
    let mut out = String::with_capacity(text.len() + 256);
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            out.push('\n');
            continue;
        }
        if t.starts_with('#') {
            let mut rest = line;
            let mut rewritten = String::new();
            while let Some(i) = rest.find("URI=\"") {
                let start = i + 5;
                let Some(len) = rest[start..].find('"') else {
                    break;
                };
                rewritten.push_str(&rest[..start]);
                rewritten.push_str(&abs(&rest[start..start + len]));
                rest = &rest[start + len..];
            }
            rewritten.push_str(rest);
            out.push_str(&rewritten);
        } else {
            out.push_str(&abs(t));
        }
        out.push('\n');
    }
    out
}

pub fn looks_like_hls(content_type: &str, url: &str, head: &[u8]) -> bool {
    let ct = content_type.to_ascii_lowercase();
    ct.contains("mpegurl")
        || url
            .split('?')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .ends_with(".m3u8")
        || head.starts_with(b"#EXTM3U")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_extended_m3u_with_attrs() {
        let t = "#EXTM3U\n#EXTINF:-1 tvg-logo=\"http://x/l.png\" group-title=\"Jazz, Soul\",Radio One\nhttp://stream.example/one\n#EXTINF:215,Artist - Song\nmusic/song.mp3\n";
        let e = parse_m3u(t, Some(Path::new("/lib")));
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].title.as_deref(), Some("Radio One"));
        assert_eq!(e[0].logo.as_deref(), Some("http://x/l.png"));
        assert_eq!(e[0].group.as_deref(), Some("Jazz, Soul"));
        assert_eq!(e[0].duration_s, None);
        assert!(e[0].is_url());
        assert_eq!(e[1].duration_s, Some(215.0));
        assert!(e[1].location.ends_with("music/song.mp3"));
        assert!(e[1].location.starts_with("/lib"));
    }

    #[test]
    fn parses_pls() {
        let t = "[playlist]\nFile1=http://a/stream\nTitle1=A\nLength1=-1\nFile2=/x/b.flac\nNumberOfEntries=2\n";
        let e = parse_any(t, "x.pls", None);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].title.as_deref(), Some("A"));
        assert_eq!(e[1].location, "/x/b.flac");
    }

    #[test]
    fn writes_relative_paths() {
        let e = vec![PlaylistEntry {
            location: "/lib/a/b.mp3".into(),
            title: Some("B".into()),
            duration_s: Some(60.4),
            logo: None,
            group: None,
        }];
        let s = write_m3u(&e, Some(Path::new("/lib")));
        assert!(s.contains("#EXTINF:60,B\na/b.mp3\n"));
    }

    #[test]
    fn absolutizes_hls_against_final_url() {
        let base = url::Url::parse("https://cdn.example.com/radio/live/index.m3u8?t=1").unwrap();
        let text = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n#EXTINF:6,\nseg1.aac\nhttps://other.example/seg2.aac\n../up/seg3.aac\n";
        let out = absolutize_hls(text, &base);
        assert!(out.contains("URI=\"https://cdn.example.com/radio/live/key.bin\""));
        assert!(out.contains("\nhttps://cdn.example.com/radio/live/seg1.aac\n"));
        assert!(out.contains("\nhttps://other.example/seg2.aac\n"));
        assert!(out.contains("\nhttps://cdn.example.com/radio/up/seg3.aac\n"));
        assert!(looks_like_hls("application/vnd.apple.mpegurl", "x", b""));
        assert!(looks_like_hls("", "https://a/b.m3u8?x=1", b""));
    }

    #[test]
    fn decodes_file_urls() {
        let e = parse_m3u("file:///home/me/My%20Song.mp3\n", None);
        assert_eq!(e[0].location, "/home/me/My Song.mp3");
    }
}
