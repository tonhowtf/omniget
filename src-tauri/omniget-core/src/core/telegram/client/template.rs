//! File name templates for every download (`tdl dl --template` parity).
//!
//! Variables: `{chat}`, `{chat_id}`, `{msg_id}`, `{date}` (YYYY-MM-DD),
//! `{time}` (HH-MM-SS), `{file_name}` (with extension), `{stem}` (without),
//! `{ext}` (no dot), `{type}`, `{caption}` (first 60 chars), `{album}`
//! (grouped id, empty when the message is not in an album). Slashes in the
//! template create subfolders; slashes inside values never do. When the last
//! segment does not end with the file's extension, it is appended.

use std::path::{Path, PathBuf};

use chrono::TimeZone;

/// What a template can refer to.
#[derive(Debug, Clone, Default)]
pub struct NameCtx {
    pub chat: String,
    pub chat_id: i64,
    pub msg_id: i32,
    /// Unix seconds; 0 when unknown.
    pub date: i64,
    pub file_name: String,
    pub media_type: String,
    pub caption: Option<String>,
    pub grouped_id: Option<i64>,
}

/// The layout used by chat downloads and auto-save when no template is set.
pub const CHAT_DEFAULT: &str = "{chat}/{msg_id}_{file_name}";

const CAPTION_CHARS: usize = 60;
const SEGMENT_MAX_BYTES: usize = 180;

/// Characters no file system we ship on accepts, plus the path separators.
fn is_bad_char(c: char) -> bool {
    matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*') || c.is_control()
}

/// One value, made safe to sit inside a single path segment.
pub fn sanitize_value(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_whitespace() {
                ' '
            } else if is_bad_char(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clean_segment(seg: &str) -> Option<String> {
    let s: String = seg.chars().filter(|c| !c.is_control()).collect();
    let s = s.trim().trim_end_matches(['.', ' ']).to_string();
    if s.is_empty() || s == "." || s == ".." {
        return None;
    }
    if s.len() <= SEGMENT_MAX_BYTES {
        return Some(s);
    }
    // Keep the extension when cutting a long name.
    let (stem, ext) = split_ext(&s);
    let budget = SEGMENT_MAX_BYTES.saturating_sub(ext.len() + 1).max(16);
    let mut cut = budget.min(stem.len());
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    let stem = stem[..cut].trim_end();
    Some(if ext.is_empty() {
        stem.to_string()
    } else {
        format!("{}.{}", stem, ext)
    })
}

/// `("movie.part1", "mp4")` for `movie.part1.mp4`; `("README", "")` otherwise.
pub fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() && name.len() - i - 1 <= 8 => {
            let ext = &name[i + 1..];
            if ext.chars().all(|c| c.is_ascii_alphanumeric()) {
                (&name[..i], ext)
            } else {
                (name, "")
            }
        }
        _ => (name, ""),
    }
}

/// Lower-case extension without the dot.
pub fn ext_of(name: &str) -> String {
    split_ext(name).1.to_ascii_lowercase()
}

fn local_datetime(ts: i64) -> Option<chrono::DateTime<chrono::Local>> {
    if ts <= 0 {
        return None;
    }
    chrono::Local.timestamp_opt(ts, 0).single()
}

fn value_of(var: &str, ctx: &NameCtx) -> Option<String> {
    let (stem, ext) = split_ext(&ctx.file_name);
    let v = match var {
        "chat" => {
            if ctx.chat.trim().is_empty() {
                ctx.chat_id.to_string()
            } else {
                ctx.chat.clone()
            }
        }
        "chat_id" => ctx.chat_id.to_string(),
        "msg_id" | "message_id" | "id" => ctx.msg_id.to_string(),
        "date" => local_datetime(ctx.date)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "0000-00-00".into()),
        "time" => local_datetime(ctx.date)
            .map(|d| d.format("%H-%M-%S").to_string())
            .unwrap_or_else(|| "00-00-00".into()),
        "file_name" | "filename" => ctx.file_name.clone(),
        "stem" => stem.to_string(),
        "ext" => ext.to_string(),
        "type" => ctx.media_type.clone(),
        "caption" => ctx
            .caption
            .as_deref()
            .map(|c| c.chars().take(CAPTION_CHARS).collect::<String>())
            .unwrap_or_default(),
        "album" => ctx.grouped_id.map(|g| g.to_string()).unwrap_or_default(),
        _ => return None,
    };
    Some(sanitize_value(&v))
}

/// The relative path a template gives for one file. Never empty, never
/// escapes its root.
pub fn render(template: &str, ctx: &NameCtx) -> PathBuf {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let var = after[..close].trim();
                match value_of(var, ctx) {
                    Some(v) => out.push_str(&v),
                    None => {
                        out.push('{');
                        out.push_str(&after[..close]);
                        out.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);

    let mut segments: Vec<String> = out
        .split(['/', '\\'])
        .filter_map(clean_segment)
        .map(|s| {
            s.chars()
                .map(|c| if is_bad_char(c) { '_' } else { c })
                .collect()
        })
        .collect();
    if segments.is_empty() {
        segments.push(
            clean_segment(&sanitize_value(&ctx.file_name))
                .unwrap_or_else(|| format!("{}", ctx.msg_id)),
        );
    }
    let ext = ext_of(&ctx.file_name);
    if !ext.is_empty() {
        if let Some(last) = segments.last_mut() {
            if !last.to_ascii_lowercase().ends_with(&format!(".{}", ext)) {
                last.push('.');
                last.push_str(&ext);
            }
        }
    }
    segments.iter().collect()
}

/// `root/<template>` when a template is given, else `root/<default>`.
/// `default` may itself be a template (chat downloads) or `None` for the
/// plain `root/<file name>` layout the single and batch downloads used.
pub fn resolve(
    root: &Path,
    template: Option<&str>,
    default: Option<&str>,
    ctx: &NameCtx,
) -> PathBuf {
    match template.filter(|t| !t.trim().is_empty()).or(default) {
        Some(t) => root.join(render(t, ctx)),
        None => root.join(sanitize_value(&ctx.file_name)),
    }
}

/// True when the template needs message data beyond id and file name.
pub fn needs_metadata(template: &str) -> bool {
    [
        "{date}",
        "{time}",
        "{caption}",
        "{album}",
        "{type}",
        "{chat}",
    ]
    .iter()
    .any(|v| template.contains(v))
}

/// `name (1).ext`, `name (2).ext`, … until the path is free.
pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, ext) = split_ext(&name);
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    for n in 1..10_000 {
        let candidate = if ext.is_empty() {
            parent.join(format!("{} ({})", stem, n))
        } else {
            parent.join(format!("{} ({}).{}", stem, n, ext))
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> NameCtx {
        NameCtx {
            chat: "My/Chat".into(),
            chat_id: 42,
            msg_id: 7,
            date: 0,
            file_name: "clip.mp4".into(),
            media_type: "video".into(),
            caption: Some("hello: world\nsecond".into()),
            grouped_id: None,
        }
    }

    #[test]
    fn renders_folders_and_sanitizes_values() {
        let p = render("{chat}/{msg_id}_{file_name}", &ctx());
        assert_eq!(p, PathBuf::from("My_Chat").join("7_clip.mp4"));
    }

    #[test]
    fn appends_missing_extension_and_blocks_escape() {
        let p = render("../{caption}", &ctx());
        assert_eq!(p, PathBuf::from("hello_ world second.mp4"));
        let p = render("{stem}.{ext}", &ctx());
        assert_eq!(p, PathBuf::from("clip.mp4"));
        let p = render("", &ctx());
        assert_eq!(p, PathBuf::from("clip.mp4"));
    }

    #[test]
    fn keeps_unknown_variables() {
        let p = render("{nope}_{msg_id}", &ctx());
        assert_eq!(p, PathBuf::from("{nope}_7.mp4"));
    }
}
