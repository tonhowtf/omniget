use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NoteDoc {
    pub title: String,
    pub source: String,
    pub course: Option<String>,
    pub module: Option<String>,
    pub lesson: Option<String>,
    pub duration_ms: Option<u64>,
    pub notes: Vec<NoteLine>,
    pub link_base: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NoteLine {
    pub t_ms: u64,
    pub t_end_ms: Option<u64>,
    pub text: String,
    pub image_rel: Option<String>,
    pub quote: Option<String>,
}

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

fn md_line(s: &str) -> String {
    s.trim().replace('\r', "")
}

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
