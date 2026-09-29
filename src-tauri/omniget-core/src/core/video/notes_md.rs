//! Markdown export of timestamped notes: YAML front matter plus one list item
//! per note linking back into the player (`omniget://watch?...&t=754`).

use serde::{Deserialize, Serialize};

/// Everything needed to export the notes of one video.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NoteDoc {
    pub title: String,
    /// File path or page URL of the video.
    pub source: String,
    pub course: Option<String>,
    pub module: Option<String>,
    pub lesson: Option<String>,
    pub duration_ms: Option<u64>,
    pub notes: Vec<NoteLine>,
    /// Deep link without the time, e.g. `omniget://watch?key=local%3Aabc`.
    pub link_base: String,
}

/// One note.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NoteLine {
    pub t_ms: u64,
    /// End of a range note.
    pub t_end_ms: Option<u64>,
    pub text: String,
    /// Relative path of the captured frame, shown under the note.
    pub image_rel: Option<String>,
    /// Transcript excerpt the note was made from.
    pub quote: Option<String>,
}

/// Formats milliseconds as `12:34` or `1:02:03`.
pub fn fmt_ts(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, sec) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

fn yaml_str(s: &str) -> String {
    let escaped = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}

/// The link for a time: `link_base` plus `t=<seconds>`.
pub fn time_link(link_base: &str, t_ms: u64) -> String {
    let sep = if link_base.contains('?') {
        if link_base.ends_with('?') || link_base.ends_with('&') {
            ""
        } else {
            "&"
        }
    } else {
        "?"
    };
    format!("{link_base}{sep}t={}", t_ms / 1000)
}

/// YouTube video id of a watch/short/embed/youtu.be URL.
fn youtube_id(url: &str) -> Option<String> {
    let u = url::Url::parse(url.trim()).ok()?;
    let host = u
        .host_str()?
        .trim_start_matches("www.")
        .trim_start_matches("m.");
    let id = match host {
        "youtu.be" => u.path_segments()?.next()?.to_string(),
        "youtube.com" | "music.youtube.com" => {
            let mut segs = u.path_segments()?;
            match segs.next()? {
                "watch" => u.query_pairs().find(|(k, _)| k == "v")?.1.into_owned(),
                "shorts" | "embed" | "live" | "v" => segs.next()?.to_string(),
                _ => return None,
            }
        }
        _ => return None,
    };
    (!id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then_some(id)
}

/// Wraps a link target in `<>` only when it has spaces or parentheses.
fn md_url(u: &str) -> String {
    let u = u.trim();
    if u.chars()
        .any(|c| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>'))
    {
        format!("<{}>", u.replace(['<', '>'], ""))
    } else {
        u.to_string()
    }
}

/// Escapes characters that would break the link label or the list item.
fn md_line(s: &str) -> String {
    s.trim().replace('\r', "")
}

/// Renders the notes as Markdown with YAML front matter. Notes are sorted by
/// time; multi-line notes stay inside their list item.
pub fn export(doc: &NoteDoc) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("title: {}\n", yaml_str(&doc.title)));
    out.push_str(&format!("source: {}\n", yaml_str(&doc.source)));
    for (k, v) in [
        ("course", &doc.course),
        ("module", &doc.module),
        ("lesson", &doc.lesson),
    ] {
        if let Some(v) = v.as_deref().filter(|v| !v.trim().is_empty()) {
            out.push_str(&format!("{k}: {}\n", yaml_str(v)));
        }
    }
    if let Some(d) = doc.duration_ms.filter(|d| *d > 0) {
        out.push_str(&format!("duration: {}\n", yaml_str(&fmt_ts(d))));
    }
    out.push_str("---\n\n");
    let heading = doc.title.trim();
    if !heading.is_empty() {
        out.push_str(&format!("# {}\n\n", heading.replace('\n', " ")));
    }
    let yt = youtube_id(&doc.source);
    let mut notes: Vec<&NoteLine> = doc.notes.iter().collect();
    notes.sort_by_key(|n| (n.t_ms, n.t_end_ms));
    for n in notes {
        let label = match n.t_end_ms.filter(|e| *e > n.t_ms) {
            Some(end) => format!("{}–{}", fmt_ts(n.t_ms), fmt_ts(end)),
            None => fmt_ts(n.t_ms),
        };
        let link = time_link(&doc.link_base, n.t_ms);
        let text = md_line(&n.text);
        let mut lines = text.lines();
        let first = lines.next().unwrap_or("");
        out.push_str(&format!("- [{label}]({})", md_url(&link)));
        if let Some(id) = &yt {
            out.push_str(&format!(
                " ([YouTube](https://youtu.be/{id}?t={}))",
                n.t_ms / 1000
            ));
        }
        if !first.is_empty() {
            out.push(' ');
            out.push_str(first);
        }
        out.push('\n');
        for l in lines {
            out.push_str("  ");
            out.push_str(l.trim_end());
            out.push('\n');
        }
        if let Some(img) = n.image_rel.as_deref().filter(|i| !i.trim().is_empty()) {
            out.push_str(&format!("\n  ![{label}]({})\n", md_url(img)));
        }
        if let Some(q) = n.quote.as_deref().filter(|q| !q.trim().is_empty()) {
            out.push('\n');
            for l in q.trim().lines() {
                out.push_str("  > ");
                out.push_str(l.trim());
                out.push('\n');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps() {
        assert_eq!(fmt_ts(0), "0:00");
        assert_eq!(fmt_ts(754_000), "12:34");
        assert_eq!(fmt_ts(754_999), "12:34");
        assert_eq!(fmt_ts(3_723_000), "1:02:03");
        assert_eq!(fmt_ts(36_000_000), "10:00:00");
    }

    #[test]
    fn links() {
        assert_eq!(
            time_link("omniget://watch?key=a", 754_500),
            "omniget://watch?key=a&t=754"
        );
        assert_eq!(time_link("omniget://watch", 1000), "omniget://watch?t=1");
        assert_eq!(time_link("omniget://watch?", 1000), "omniget://watch?t=1");
        assert_eq!(
            youtube_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=3").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(
            youtube_id("https://youtu.be/dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(youtube_id("/Users/me/a.mp4"), None);
    }

    #[test]
    fn export_markdown() {
        let doc = NoteDoc {
            title: "Aula 3: \"Closures\"".into(),
            source: "/cursos/rust/03.mp4".into(),
            course: Some("Rust".into()),
            module: Some("Módulo 2".into()),
            lesson: Some("3".into()),
            duration_ms: Some(1_800_000),
            link_base: "omniget://watch?key=local%3Aabc".into(),
            notes: vec![
                NoteLine {
                    t_ms: 754_000,
                    t_end_ms: Some(790_000),
                    text: "Trecho importante\nsegunda linha".into(),
                    image_rel: Some("anexos/aula 03-12m34s.png".into()),
                    quote: Some("o que foi dito".into()),
                },
                NoteLine {
                    t_ms: 5_000,
                    text: "primeira".into(),
                    ..NoteLine::default()
                },
            ],
        };
        let md = export(&doc);
        let expected = "---\n\
title: \"Aula 3: \\\"Closures\\\"\"\n\
source: \"/cursos/rust/03.mp4\"\n\
course: \"Rust\"\n\
module: \"Módulo 2\"\n\
lesson: \"3\"\n\
duration: \"30:00\"\n\
---\n\
\n\
# Aula 3: \"Closures\"\n\
\n\
- [0:05](omniget://watch?key=local%3Aabc&t=5) primeira\n\
- [12:34–13:10](omniget://watch?key=local%3Aabc&t=754) Trecho importante\n  segunda linha\n\
\n  ![12:34–13:10](<anexos/aula 03-12m34s.png>)\n\
\n  > o que foi dito\n";
        assert_eq!(md, expected);
    }

    #[test]
    fn youtube_source_and_empty() {
        let doc = NoteDoc {
            title: String::new(),
            source: "https://youtu.be/dQw4w9WgXcQ".into(),
            link_base: "omniget://watch?key=url%3Ax".into(),
            notes: vec![NoteLine {
                t_ms: 61_000,
                text: "x".into(),
                ..NoteLine::default()
            }],
            ..NoteDoc::default()
        };
        let md = export(&doc);
        assert!(md.contains(
            "- [1:01](omniget://watch?key=url%3Ax&t=61) ([YouTube](https://youtu.be/dQw4w9WgXcQ?t=61)) x\n"
        ));
        assert!(!md.contains("course:"));
        assert!(export(&NoteDoc::default()).starts_with("---\ntitle: \"\"\n"));
    }
}
