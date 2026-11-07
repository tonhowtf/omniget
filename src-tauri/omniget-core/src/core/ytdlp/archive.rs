use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct ArchiveEntry {
    pub extractor: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveParseError {
    pub line: usize,
    pub content: String,
}

impl std::fmt::Display for ArchiveParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ArchiveInvalidLine|{}|{}",
            self.line,
            self.content.chars().take(80).collect::<String>()
        )
    }
}

impl std::error::Error for ArchiveParseError {}

fn valid_extractor(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 64
        && token
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn valid_id(token: &str) -> bool {
    !token.is_empty() && token.len() <= 256 && !token.chars().any(|c| c.is_control())
}

pub fn parse_archive(text: &str) -> Result<Vec<ArchiveEntry>, ArchiveParseError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let [extractor, id] = tokens.as_slice() else {
            return Err(ArchiveParseError {
                line: index + 1,
                content: line.to_string(),
            });
        };
        if !valid_extractor(extractor) || !valid_id(id) {
            return Err(ArchiveParseError {
                line: index + 1,
                content: line.to_string(),
            });
        }
        let entry = ArchiveEntry {
            extractor: (*extractor).to_string(),
            id: (*id).to_string(),
        };
        if seen.insert(entry.clone()) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

pub fn format_archive(entries: &[ArchiveEntry]) -> String {
    let mut out = String::with_capacity(entries.len() * 24);
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        if seen.insert(entry) {
            out.push_str(&entry.extractor);
            out.push(' ');
            out.push_str(&entry.id);
            out.push('\n');
        }
    }
    out
}

fn path_segments(parsed: &url::Url) -> Vec<&str> {
    parsed
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default()
}

pub fn archive_entry_for_url(url: &str) -> Option<ArchiveEntry> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let is = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    let segments = path_segments(&parsed);
    let (extractor, id): (&str, Option<String>) = if is("youtube.com") || host == "youtu.be" {
        let id = if host == "youtu.be" {
            segments.first().map(|s| s.to_string())
        } else if matches!(segments.first(), Some(&"shorts" | &"live" | &"embed")) {
            segments.get(1).map(|s| s.to_string())
        } else {
            parsed
                .query_pairs()
                .find(|(k, _)| k == "v")
                .map(|(_, v)| v.into_owned())
        };
        ("youtube", id)
    } else if is("vimeo.com") {
        (
            "vimeo",
            segments
                .iter()
                .rev()
                .find(|s| s.chars().all(|c| c.is_ascii_digit()))
                .map(|s| s.to_string()),
        )
    } else if is("tiktok.com") {
        (
            "tiktok",
            segments
                .iter()
                .position(|s| *s == "video")
                .and_then(|i| segments.get(i + 1))
                .map(|s| s.to_string()),
        )
    } else if is("instagram.com") {
        (
            "instagram",
            segments
                .iter()
                .position(|s| matches!(*s, "p" | "reel" | "reels" | "tv"))
                .and_then(|i| segments.get(i + 1))
                .map(|s| s.to_string()),
        )
    } else if is("twitter.com") || is("x.com") {
        (
            "twitter",
            segments
                .iter()
                .position(|s| *s == "status")
                .and_then(|i| segments.get(i + 1))
                .map(|s| s.to_string()),
        )
    } else {
        return None;
    };
    let id = id.filter(|i| valid_id(i) && !i.contains('[') && !i.contains(' '))?;
    Some(ArchiveEntry {
        extractor: extractor.to_string(),
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_extractor_and_id_of_every_line() {
        let parsed = parse_archive("youtube abc123\r\nvimeo 42\n\n").unwrap();
        assert_eq!(
            parsed,
            vec![
                ArchiveEntry {
                    extractor: "youtube".into(),
                    id: "abc123".into()
                },
                ArchiveEntry {
                    extractor: "vimeo".into(),
                    id: "42".into()
                },
            ]
        );
    }

    #[test]
    fn repeated_lines_count_once() {
        assert_eq!(
            parse_archive("youtube abc\nyoutube abc\n").unwrap().len(),
            1
        );
    }

    #[test]
    fn a_line_with_three_tokens_rejects_the_file_and_names_the_line() {
        let err = parse_archive("youtube abc\nyoutube abc extra\n").unwrap_err();
        assert_eq!(err.line, 2);
    }

    #[test]
    fn a_line_with_one_token_rejects_the_file() {
        assert_eq!(parse_archive("abc123\n").unwrap_err().line, 1);
    }

    #[test]
    fn a_comment_or_uppercase_extractor_is_not_an_archive_line() {
        assert!(parse_archive("# not an archive\n").is_err());
        assert!(parse_archive("Youtube abc\n").is_err());
        assert!(parse_archive("https://youtu.be/abc x\n").is_err());
    }

    #[test]
    fn export_writes_the_format_ytdlp_reads_back() {
        let original = "youtube abc123\nvimeo 42\n";
        let parsed = parse_archive(original).unwrap();
        assert_eq!(format_archive(&parsed), original);
    }

    #[test]
    fn archive_ids_come_from_each_supported_link_shape() {
        let id = |u: &str| archive_entry_for_url(u).map(|e| format!("{} {}", e.extractor, e.id));
        assert_eq!(
            id("https://www.youtube.com/watch?v=abc123&list=PL1").as_deref(),
            Some("youtube abc123")
        );
        assert_eq!(
            id("https://youtu.be/abc123?si=x").as_deref(),
            Some("youtube abc123")
        );
        assert_eq!(
            id("https://www.youtube.com/live/abc123").as_deref(),
            Some("youtube abc123")
        );
        assert_eq!(
            id("https://vimeo.com/channels/foo/42").as_deref(),
            Some("vimeo 42")
        );
        assert_eq!(
            id("https://www.tiktok.com/@a/video/99").as_deref(),
            Some("tiktok 99")
        );
        assert_eq!(
            id("https://www.instagram.com/reel/AbC/").as_deref(),
            Some("instagram AbC")
        );
        assert_eq!(
            id("https://x.com/a/status/77").as_deref(),
            Some("twitter 77")
        );
    }

    #[test]
    fn links_without_a_known_id_are_left_out_of_the_archive() {
        assert_eq!(archive_entry_for_url("https://example.com/a"), None);
        assert_eq!(
            archive_entry_for_url("https://www.youtube.com/playlist?list=PL1"),
            None
        );
        assert_eq!(
            archive_entry_for_url("https://www.youtube.com/watch?v=[redacted]"),
            None
        );
    }
}
