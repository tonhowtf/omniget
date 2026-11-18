use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use super::model::{media_of, snowflake_at, snowflake_ms, MediaItem, Message};
use super::rest::Rest;

const PAGE: usize = 100;
const PARALLEL: usize = 3;

#[derive(Deserialize, Clone, Debug, Default)]
pub struct MediaFilter {
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub after_ms: Option<u64>,
    #[serde(default)]
    pub before_ms: Option<u64>,
    #[serde(default = "yes")]
    pub include_embeds: bool,
}

fn yes() -> bool {
    true
}

impl MediaFilter {
    pub fn wants(&self, item: &MediaItem) -> bool {
        (self.kinds.is_empty() || self.kinds.iter().any(|k| k == item.kind))
            && self.in_range(item.timestamp_ms)
    }

    pub fn in_range(&self, ms: u64) -> bool {
        self.after_ms.is_none_or(|a| ms >= a) && self.before_ms.is_none_or(|b| ms < b)
    }
}

pub fn clean_segment(name: &str, fallback: &str) -> String {
    let opts = sanitize_filename::Options {
        truncate: true,
        windows: true,
        replacement: "_",
    };
    let s = sanitize_filename::sanitize_with_options(name.trim(), opts);
    let s = s
        .trim_matches(|c: char| c == '.' || c.is_whitespace())
        .to_string();
    if s.chars().all(|c| c == '_') {
        fallback.to_string()
    } else {
        s
    }
}

fn stamp(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|t| t.format("%Y-%m-%d_%H-%M-%S").to_string())
        .unwrap_or_else(|| "0000-00-00_00-00-00".into())
}

pub fn media_file_name(item: &MediaItem, index: usize) -> String {
    let raw = item.filename.trim();
    let (stem, ext) = match raw.rsplit_once('.') {
        Some((s, e))
            if !s.is_empty()
                && !e.is_empty()
                && e.len() <= 8
                && e.bytes().all(|b| b.is_ascii_alphanumeric()) =>
        {
            (s, format!(".{}", e.to_ascii_lowercase()))
        }
        _ => (raw, String::new()),
    };
    let stem: String = clean_segment(stem, "file").chars().take(80).collect();
    format!(
        "{}_{}_{}_{}{}",
        stamp(item.timestamp_ms),
        item.message_id,
        index + 1,
        stem,
        ext
    )
}

pub fn channel_folder(root: &Path, place: &str, channel: &str) -> PathBuf {
    root.join(super::model::STORE_LABEL)
        .join(clean_segment(place, super::model::STORE_LABEL))
        .join(clean_segment(channel, "channel"))
}

pub async fn fetch_page(
    rest: &Rest,
    channel_id: &str,
    before: Option<&str>,
) -> Result<Vec<Message>, String> {
    let path = match before {
        Some(b) => format!("/channels/{channel_id}/messages?limit={PAGE}&before={b}"),
        None => format!("/channels/{channel_id}/messages?limit={PAGE}"),
    };
    let v = rest.get(&path).await?;
    Ok(Message::list_from(&v))
}

pub async fn walk_history<F>(
    rest: &Rest,
    channel_id: &str,
    after_ms: Option<u64>,
    before_ms: Option<u64>,
    cancel: &CancellationToken,
    mut on_page: F,
) -> Result<bool, String>
where
    F: FnMut(Vec<Message>),
{
    let mut before = before_ms.map(|ms| snowflake_at(ms).to_string());
    loop {
        if cancel.is_cancelled() {
            return Ok(false);
        }
        let page = fetch_page(rest, channel_id, before.as_deref()).await?;
        let Some(last) = page.last() else {
            return Ok(true);
        };
        let oldest = snowflake_ms(&last.id).unwrap_or(0);
        before = Some(last.id.clone());
        let full = page.len() >= PAGE;
        let kept: Vec<Message> = page
            .into_iter()
            .filter(|m| after_ms.is_none_or(|a| snowflake_ms(&m.id).unwrap_or(0) >= a))
            .collect();
        on_page(kept);
        if !full || after_ms.is_some_and(|a| oldest < a) {
            return Ok(true);
        }
    }
}

pub enum Saved {
    Written(PathBuf, u64),
    Skipped(PathBuf),
}

pub async fn save_url(
    http: &reqwest::Client,
    url: &str,
    target: &Path,
    cancel: &CancellationToken,
) -> Result<Saved, String> {
    if !url.starts_with("https://") {
        return Err("unsupported url".into());
    }
    if tokio::fs::metadata(target).await.is_ok() {
        return Ok(Saved::Skipped(target.to_path_buf()));
    }
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| format!("network: {}", e.without_url()))?;
    if !resp.status().is_success() {
        return Err(format!("http_{}", resp.status().as_u16()));
    }
    let part = target.with_extension(format!(
        "{}part",
        target
            .extension()
            .map(|e| format!("{}.", e.to_string_lossy()))
            .unwrap_or_default()
    ));
    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(|e| e.to_string())?;
    let mut stream = resp.bytes_stream();
    let mut written = 0u64;
    while let Some(chunk) = stream.next().await {
        if cancel.is_cancelled() {
            drop(file);
            let _ = tokio::fs::remove_file(&part).await;
            return Err("cancelled".into());
        }
        let chunk = chunk.map_err(|e| format!("network: {}", e.without_url()))?;
        written += chunk.len() as u64;
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    tokio::fs::rename(&part, target)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Saved::Written(target.to_path_buf(), written))
}

#[derive(Default)]
pub struct Progress {
    pub scanned: AtomicUsize,
    pub found: AtomicUsize,
    pub done: AtomicUsize,
    pub skipped: AtomicUsize,
    pub failed: AtomicUsize,
    pub bytes: AtomicU64,
}

impl Progress {
    pub fn new() -> Self {
        Self {
            scanned: AtomicUsize::new(0),
            found: AtomicUsize::new(0),
            done: AtomicUsize::new(0),
            skipped: AtomicUsize::new(0),
            failed: AtomicUsize::new(0),
            bytes: AtomicU64::new(0),
        }
    }

    pub fn json(&self, job_id: &str, phase: &str) -> Value {
        json!({
            "job_id": job_id,
            "phase": phase,
            "scanned": self.scanned.load(Ordering::Relaxed),
            "found": self.found.load(Ordering::Relaxed),
            "done": self.done.load(Ordering::Relaxed),
            "skipped": self.skipped.load(Ordering::Relaxed),
            "failed": self.failed.load(Ordering::Relaxed),
            "bytes": self.bytes.load(Ordering::Relaxed),
        })
    }
}

pub async fn save_items(
    http: &reqwest::Client,
    items: Vec<(MediaItem, PathBuf)>,
    progress: &Arc<Progress>,
    cancel: &CancellationToken,
    on_step: &(dyn Fn() + Send + Sync),
) -> Vec<PathBuf> {
    let results: Vec<Option<PathBuf>> = futures::stream::iter(items)
        .map(|(item, path)| {
            let http = http.clone();
            let cancel = cancel.clone();
            let progress = progress.clone();
            async move {
                if cancel.is_cancelled() {
                    return None;
                }
                let r = save_url(&http, &item.url, &path, &cancel).await;
                let out = match r {
                    Ok(Saved::Written(p, n)) => {
                        progress.done.fetch_add(1, Ordering::Relaxed);
                        progress.bytes.fetch_add(n, Ordering::Relaxed);
                        Some(p)
                    }
                    Ok(Saved::Skipped(p)) => {
                        progress.skipped.fetch_add(1, Ordering::Relaxed);
                        Some(p)
                    }
                    Err(e) => {
                        if e != "cancelled" {
                            tracing::warn!("[guildchat] media download failed: {}", e);
                            progress.failed.fetch_add(1, Ordering::Relaxed);
                        }
                        None
                    }
                };
                on_step();
                out
            }
        })
        .buffer_unordered(PARALLEL)
        .collect()
        .await;
    results.into_iter().flatten().collect()
}

pub fn plan(
    messages: &[Message],
    filter: &MediaFilter,
    folder: &Path,
) -> Vec<(MediaItem, PathBuf)> {
    let mut out = Vec::new();
    for m in messages {
        for (i, item) in media_of(m, filter.include_embeds).into_iter().enumerate() {
            if filter.wants(&item) {
                let path = folder.join(media_file_name(&item, i));
                out.push((item, path));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, kind: &'static str, ms: u64) -> MediaItem {
        MediaItem {
            url: "https://cdn.discordapp.com/x".into(),
            filename: name.into(),
            kind,
            size: 1,
            message_id: "42".into(),
            timestamp_ms: ms,
        }
    }

    #[test]
    fn file_names_are_unique_dated_and_safe() {
        let ms = 1_762_432_205_000;
        assert_eq!(
            media_file_name(&item("cat.png", "image", ms), 0),
            "2025-11-06_12-30-05_42_1_cat.png"
        );
        assert_eq!(
            media_file_name(&item("cat.png", "image", ms), 1),
            "2025-11-06_12-30-05_42_2_cat.png"
        );
        let evil = media_file_name(&item("../../etc/pa:ss?.txt", "file", ms), 0);
        assert!(!evil.contains('/') && !evil.contains(':') && !evil.contains('?'));
        assert!(evil.ends_with(".txt"));
        let long = media_file_name(&item(&format!("{}.mp4", "a".repeat(300)), "video", ms), 0);
        assert!(long.len() < 140);
        assert!(long.ends_with(".mp4"));
        assert!(media_file_name(&item("", "file", ms), 0).ends_with("_file"));
    }

    #[test]
    fn folders_are_sanitized() {
        let p = channel_folder(Path::new("/dl"), "My/Guild", "..");
        assert_eq!(p, Path::new("/dl/Discord/My_Guild/channel"));
        assert_eq!(clean_segment("   ", "x"), "x");
    }

    #[test]
    fn filter_by_kind_and_range() {
        let f = MediaFilter {
            kinds: vec!["image".into()],
            after_ms: Some(100),
            before_ms: Some(200),
            include_embeds: true,
        };
        assert!(f.wants(&item("a.png", "image", 150)));
        assert!(!f.wants(&item("a.mp4", "video", 150)));
        assert!(!f.wants(&item("a.png", "image", 99)));
        assert!(!f.wants(&item("a.png", "image", 200)));
        assert!(MediaFilter::default().wants(&item("a.mp4", "video", 0)));
    }
}
