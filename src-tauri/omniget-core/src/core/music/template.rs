//! File name templates for renaming and conversion output:
//! `{album_artist}/{album}/{disc}-{track} {title}`. A `/` in the template
//! makes folders; every tag value is sanitized so it can't.

use super::tags::TrackMeta;

pub const DEFAULT_TEMPLATE: &str = "{album_artist}/{album}/{track} - {title}";

fn value(m: &TrackMeta, key: &str) -> String {
    let or_unknown = |s: &str, fallback: &str| {
        if s.trim().is_empty() {
            fallback.to_string()
        } else {
            s.trim().to_string()
        }
    };
    match key {
        "title" => or_unknown(&m.title, "Unknown Title"),
        "artist" => or_unknown(&m.artist, "Unknown Artist"),
        "album" => or_unknown(&m.album, "Unknown Album"),
        "album_artist" => {
            if m.album_artist.trim().is_empty() {
                or_unknown(&m.artist, "Unknown Artist")
            } else {
                m.album_artist.trim().to_string()
            }
        }
        "genre" => or_unknown(&m.genre, "Unknown Genre"),
        "year" => m.year.map(|y| y.to_string()).unwrap_or_default(),
        "track" => m
            .track
            .map(|t| format!("{t:02}"))
            .unwrap_or_else(|| "00".into()),
        "disc" => m.disc.map(|d| d.to_string()).unwrap_or_else(|| "1".into()),
        "bpm" => m
            .bpm
            .map(|b| format!("{}", b.round() as i64))
            .unwrap_or_default(),
        "key" => m.musical_key.clone(),
        "codec" => m.codec.to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn clean(component: &str) -> String {
    let s = sanitize_filename::sanitize_with_options(
        component,
        sanitize_filename::Options {
            windows: true,
            truncate: true,
            replacement: "_",
        },
    );
    // no hidden files, no trailing dots/spaces (Windows)
    let s = s
        .trim()
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .to_string();
    if s.is_empty() {
        "_".into()
    } else {
        s
    }
}

/// Relative path (with `/` separators) without extension.
pub fn render(template: &str, m: &TrackMeta) -> String {
    let template = if template.trim().is_empty() {
        DEFAULT_TEMPLATE
    } else {
        template
    };
    let mut parts = Vec::new();
    for seg in template.split(['/', '\\']) {
        let mut out = String::new();
        let mut rest = seg;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            match rest[open..].find('}') {
                Some(close) => {
                    let key = &rest[open + 1..open + close];
                    out.push_str(&value(m, key.trim()));
                    rest = &rest[open + close + 1..];
                }
                None => {
                    out.push_str(&rest[open..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        let c = clean(&out);
        if c != "_" || !out.trim().is_empty() {
            parts.push(c);
        }
    }
    parts.retain(|p| p != ".." && !p.is_empty());
    if parts.is_empty() {
        parts.push(clean(&value(m, "title")));
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> TrackMeta {
        TrackMeta {
            title: "Song: Part 1/2".into(),
            artist: "Band".into(),
            album: "Album".into(),
            track: Some(3),
            year: Some(1999),
            ..Default::default()
        }
    }

    #[test]
    fn renders_folders_and_sanitizes_values() {
        let p = render(DEFAULT_TEMPLATE, &meta());
        assert_eq!(p, "Band/Album/03 - Song_ Part 1_2");
    }

    #[test]
    fn falls_back_for_missing_tags() {
        let p = render(
            "{genre}/{year} {title}",
            &TrackMeta {
                title: "x".into(),
                ..Default::default()
            },
        );
        assert_eq!(p, "Unknown Genre/x");
    }

    #[test]
    fn blocks_parent_traversal() {
        let p = render("../../{title}", &meta());
        assert!(!p.contains(".."));
    }
}
