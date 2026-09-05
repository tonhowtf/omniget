//! X / Twitter (estudo 67): modelo compartilhado e utilidades da categoria X.
//!
//! Duas vias para tudo: a pública (FxTwitter v2, sem login) e a da sessão
//! (GraphQL interno do X com os cookies do bucket `x.com`). Cada ferramenta
//! escolhe a via; o modelo de post/usuário é o mesmo nas duas.

pub mod archive;
pub mod bookmarks;
pub mod client;
pub mod export;
pub mod follows;
pub mod fx;
pub mod grok;
pub mod media;
pub mod parse;
pub mod profile;
pub mod query_ids;
pub mod search;
pub mod thread;
pub mod txid;

use std::collections::HashSet;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

pub use super::{report, ProgressFn};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XUser {
    pub id: String,
    pub handle: String,
    pub name: String,
    #[serde(default)]
    pub avatar: String,
    #[serde(default)]
    pub banner: String,
    #[serde(default)]
    pub bio: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub website: String,
    #[serde(default)]
    pub joined: String,
    #[serde(default)]
    pub followers: u64,
    #[serde(default)]
    pub following: u64,
    #[serde(default)]
    pub posts: u64,
    #[serde(default)]
    pub likes: u64,
    #[serde(default)]
    pub media_count: u64,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub protected: bool,
    /// Eu sigo esta conta (só vem das listas da sessão).
    #[serde(default)]
    pub followed_by_me: Option<bool>,
    /// Esta conta me segue (só vem das listas da sessão).
    #[serde(default)]
    pub follows_me: Option<bool>,
}

impl XUser {
    pub fn url(&self) -> String {
        format!("https://x.com/{}", self.handle)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XMedia {
    /// "photo" | "video" | "gif"
    pub kind: String,
    /// Melhor URL: `?name=orig` para fotos, mp4 de maior bitrate para vídeos.
    pub url: String,
    #[serde(default)]
    pub thumb: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub alt: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XPost {
    pub id: String,
    pub url: String,
    pub text: String,
    /// ISO 8601 (UTC).
    pub created_at: String,
    pub timestamp: i64,
    pub author: XUser,
    #[serde(default)]
    pub likes: u64,
    #[serde(default)]
    pub reposts: u64,
    #[serde(default)]
    pub replies: u64,
    #[serde(default)]
    pub quotes: u64,
    #[serde(default)]
    pub views: u64,
    #[serde(default)]
    pub bookmarks: u64,
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub media: Vec<XMedia>,
    #[serde(default)]
    pub quote: Option<Box<XPost>>,
    #[serde(default)]
    pub reply_to_id: Option<String>,
    #[serde(default)]
    pub reply_to_handle: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// Quem repostou, quando o item da timeline é um repost.
    #[serde(default)]
    pub reposted_by: Option<String>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub hashtags: Vec<String>,
    #[serde(default)]
    pub mentions: Vec<String>,
    #[serde(default)]
    pub links: Vec<String>,
}

impl XPost {
    pub fn is_reply(&self) -> bool {
        self.reply_to_id.is_some()
    }
}

/// `https://x.com/user/status/123`, `twitter.com/.../123`, `123`.
pub fn post_id_from(input: &str) -> Option<String> {
    let s = input.trim();
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
        return Some(s.to_string());
    }
    let re = regex::Regex::new(r"(?:status(?:es)?|i/web/status)/(\d+)").ok()?;
    re.captures(s).map(|c| c[1].to_string())
}

/// `@nasa`, `nasa`, `https://x.com/nasa`, `x.com/nasa/media`.
pub fn handle_from(input: &str) -> Option<String> {
    let s = input.trim().trim_start_matches('@');
    if s.is_empty() {
        return None;
    }
    let re = regex::Regex::new(r"^[A-Za-z0-9_]{1,15}$").ok()?;
    if re.is_match(s) {
        return Some(s.to_string());
    }
    let re_url =
        regex::Regex::new(r"(?:x\.com|twitter\.com)/(?:#!/)?@?([A-Za-z0-9_]{1,15})(?:[/?#]|$)")
            .ok()?;
    let h = re_url.captures(s).map(|c| c[1].to_string())?;
    let reserved = [
        "i",
        "home",
        "explore",
        "search",
        "settings",
        "messages",
        "notifications",
        "hashtag",
        "intent",
        "compose",
    ];
    if reserved.contains(&h.to_ascii_lowercase().as_str()) {
        return None;
    }
    Some(h)
}

/// Pasta de dados da categoria: `<app_data>/tools/x`.
pub fn x_dir() -> std::path::PathBuf {
    let d = super::tools_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("x");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Sinal de cancelamento por id de tarefa (mídia em lote, unfollow, export).
static CANCELLED: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

pub fn cancel(id: &str) {
    if let Ok(mut s) = CANCELLED.lock() {
        s.insert(id.to_string());
    }
}

pub fn cancelled(id: &str) -> bool {
    CANCELLED.lock().map(|s| s.contains(id)).unwrap_or(false)
}

pub fn clear_cancel(id: &str) {
    if let Ok(mut s) = CANCELLED.lock() {
        s.remove(id);
    }
}

/// "Wed Dec 19 20:20:32 +0000 2007" → (ISO 8601, epoch).
pub fn parse_twitter_date(s: &str) -> Option<(String, i64)> {
    let dt = chrono::DateTime::parse_from_str(s.trim(), "%a %b %d %H:%M:%S %z %Y").ok()?;
    Some((dt.to_utc().to_rfc3339(), dt.timestamp()))
}

pub fn iso_from_timestamp(ts: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0)
        .map(|d| d.to_rfc3339())
        .unwrap_or_default()
}

/// URL de foto em tamanho original (`pbs.twimg.com/media/X.jpg` → `?format=jpg&name=orig`).
pub fn photo_orig_url(url: &str) -> String {
    if url.contains("name=orig") {
        return url.to_string();
    }
    let base = url.split('?').next().unwrap_or(url);
    if let Some((stem, ext)) = base.rsplit_once('.') {
        if matches!(ext, "jpg" | "jpeg" | "png" | "webp") {
            return format!("{}?format={}&name=orig", stem, ext);
        }
    }
    url.to_string()
}

pub fn dedup_posts(posts: Vec<XPost>) -> Vec<XPost> {
    let mut seen = HashSet::new();
    posts
        .into_iter()
        .filter(|p| seen.insert(p.id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ids_and_handles() {
        assert_eq!(
            post_id_from("https://x.com/jack/status/20").as_deref(),
            Some("20")
        );
        assert_eq!(
            post_id_from("https://twitter.com/i/web/status/123456?s=20").as_deref(),
            Some("123456")
        );
        assert_eq!(post_id_from("20").as_deref(), Some("20"));
        assert_eq!(handle_from("@NASA").as_deref(), Some("NASA"));
        assert_eq!(
            handle_from("https://x.com/nasa/media").as_deref(),
            Some("nasa")
        );
        assert_eq!(handle_from("https://x.com/i/bookmarks"), None);
    }

    #[test]
    fn orig_photo_url() {
        assert_eq!(
            photo_orig_url("https://pbs.twimg.com/media/abc.jpg"),
            "https://pbs.twimg.com/media/abc?format=jpg&name=orig"
        );
        assert_eq!(
            photo_orig_url("https://pbs.twimg.com/media/abc.jpg?name=orig"),
            "https://pbs.twimg.com/media/abc.jpg?name=orig"
        );
    }

    #[test]
    fn twitter_date() {
        let (iso, ts) = parse_twitter_date("Tue Mar 21 20:50:14 +0000 2006").unwrap();
        assert_eq!(ts, 1142974214);
        assert!(iso.starts_with("2006-03-21T20:50:14"));
    }
}
