use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::TimeZone;

#[derive(Debug, Clone, Default)]
pub struct NameCtx {
    pub chat: String,
    pub chat_id: i64,
    pub msg_id: i32,
    pub date: i64,
    pub file_name: String,
    pub media_type: String,
    pub caption: Option<String>,
    pub grouped_id: Option<i64>,
}

pub const CHAT_DEFAULT: &str = "{chat}/{msg_id}_{file_name}";

const CAPTION_CHARS: usize = 60;
const SEGMENT_MAX_BYTES: usize = 180;

fn is_bad_char(c: char) -> bool {
    matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*') || c.is_control()
}

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

pub fn numbered_path(path: &Path, n: u32) -> PathBuf {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, ext) = split_ext(&name);
    let numbered = if ext.is_empty() {
        format!("{} ({})", stem, n)
    } else {
        format!("{} ({}).{}", stem, n, ext)
    };
    path.with_file_name(numbered)
}

pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    (1..10_000)
        .map(|n| numbered_path(path, n))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| path.to_path_buf())
}

fn claimed_paths() -> &'static Mutex<HashSet<PathBuf>> {
    static CLAIMED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    CLAIMED.get_or_init(|| Mutex::new(HashSet::new()))
}

#[derive(Debug)]
pub struct PathClaim(PathBuf);

impl PathClaim {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PathClaim {
    fn drop(&mut self) {
        claimed_paths()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

pub fn claim_unique_path(path: &Path) -> PathClaim {
    let mut claimed = claimed_paths().lock().unwrap_or_else(|e| e.into_inner());
    let free = |candidate: &PathBuf| !candidate.exists() && !claimed.contains(candidate);
    let chosen = std::iter::once(path.to_path_buf())
        .chain((1..100_000).map(|n| numbered_path(path, n)))
        .find(free)
        .unwrap_or_else(|| numbered_path(path, 100_000));
    claimed.insert(chosen.clone());
    PathClaim(chosen)
}

pub fn rename_without_overwriting(part: &Path, target: &Path) -> std::io::Result<PathBuf> {
    let mut n = 0;
    let mut candidate = target.to_path_buf();
    loop {
        match std::fs::hard_link(part, &candidate) {
            Ok(()) => {
                std::fs::remove_file(part)?;
                return Ok(candidate);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                n += 1;
                candidate = numbered_path(target, n);
            }
            Err(_) if !candidate.exists() => {
                std::fs::rename(part, &candidate)?;
                return Ok(candidate);
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod no_overwrite_tests {
    use super::*;

    #[test]
    fn second_file_with_the_same_name_gets_a_number() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Aula.mp4");
        std::fs::write(&path, b"first").unwrap();
        let claim = claim_unique_path(&path);
        assert_eq!(claim.path(), dir.path().join("Aula (1).mp4"));
    }

    #[test]
    fn concurrent_downloads_racing_on_one_name_never_share_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Aula.mp4");
        std::fs::write(&path, b"first").unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let (path, barrier) = (path.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    claim_unique_path(&path)
                })
            })
            .collect();
        let claims: Vec<PathClaim> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let distinct: HashSet<&Path> = claims.iter().map(PathClaim::path).collect();
        assert_eq!(distinct.len(), 16);
        assert!(!distinct.contains(path.as_path()));
        assert!(distinct.contains(dir.path().join("Aula (16).mp4").as_path()));
    }

    #[test]
    fn a_released_name_can_be_used_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Aula.mp4");
        let first = claim_unique_path(&path);
        assert_eq!(first.path(), path);
        drop(first);
        assert_eq!(claim_unique_path(&path).path(), path);
    }

    #[test]
    fn finishing_a_download_never_writes_over_a_file_that_appeared_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Aula.mp4");
        let part = dir.path().join("Aula.mp4.part");
        std::fs::write(&target, b"someone else").unwrap();
        std::fs::write(&part, b"mine").unwrap();
        let saved = rename_without_overwriting(&part, &target).unwrap();
        assert_eq!(saved, dir.path().join("Aula (1).mp4"));
        assert_eq!(std::fs::read(&target).unwrap(), b"someone else");
        assert_eq!(std::fs::read(&saved).unwrap(), b"mine");
        assert!(!part.exists());
    }
}
