use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

const DEFAULT_PER_HOST_LIMIT: usize = 4;
const YOUTUBE_PER_HOST_LIMIT: usize = 2;
const INSTAGRAM_PER_HOST_LIMIT: usize = 2;
const TIKTOK_PER_HOST_LIMIT: usize = 2;
const TWITTER_PER_HOST_LIMIT: usize = 2;
const REDDIT_PER_HOST_LIMIT: usize = 3;

const DEFAULT_REQUEST_INTERVAL_MS: u64 = 0;
const YOUTUBE_REQUEST_INTERVAL_MS: u64 = 1500;
const INSTAGRAM_REQUEST_INTERVAL_MS: u64 = 2000;
const TIKTOK_REQUEST_INTERVAL_MS: u64 = 1500;
const TWITTER_REQUEST_INTERVAL_MS: u64 = 1500;
const REDDIT_REQUEST_INTERVAL_MS: u64 = 1000;

#[derive(Default)]
struct HostState {
    semaphores: HashMap<String, Arc<Semaphore>>,
    last_dispatch: HashMap<String, std::time::Instant>,
}

fn state() -> &'static Mutex<HostState> {
    static STATE: OnceLock<Mutex<HostState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(HostState::default()))
}

pub fn host_key_for_url(url: &str) -> String {
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return "unknown".to_string(),
    };
    let host = parsed.host_str().unwrap_or("unknown").to_lowercase();
    canonicalize_host(&host)
}

fn canonicalize_host(host: &str) -> String {
    let trimmed = host.trim_start_matches("www.");
    if trimmed.contains("youtube.com")
        || trimmed.contains("youtu.be")
        || trimmed.contains("googlevideo.com")
    {
        return "youtube".to_string();
    }
    if trimmed.contains("instagram.com") || trimmed.contains("cdninstagram.com") {
        return "instagram".to_string();
    }
    if trimmed.contains("tiktok.com") || trimmed.contains("tiktokcdn.com") {
        return "tiktok".to_string();
    }
    if trimmed.contains("twitter.com") || trimmed.contains("x.com") || trimmed.contains("twimg.com")
    {
        return "twitter".to_string();
    }
    if trimmed.contains("reddit.com")
        || trimmed.contains("redd.it")
        || trimmed.contains("redditmedia.com")
    {
        return "reddit".to_string();
    }
    if trimmed.contains("vimeo.com") || trimmed.contains("vimeocdn.com") {
        return "vimeo".to_string();
    }
    if trimmed.contains("twitch.tv") || trimmed.contains("ttvnw.net") {
        return "twitch".to_string();
    }
    if trimmed.contains("bilibili.com") || trimmed.contains("bilivideo.com") {
        return "bilibili".to_string();
    }
    if trimmed.contains("bsky.app") || trimmed.contains("bsky.social") {
        return "bluesky".to_string();
    }
    if trimmed.contains("pinterest") || trimmed.contains("pinimg.com") {
        return "pinterest".to_string();
    }
    if trimmed.contains("soundcloud.com") {
        return "soundcloud".to_string();
    }
    if trimmed.contains("facebook.com") || trimmed.contains("fbcdn.net") {
        return "facebook".to_string();
    }
    trimmed.to_string()
}

fn limit_for_host(key: &str) -> usize {
    match key {
        "youtube" => YOUTUBE_PER_HOST_LIMIT,
        "instagram" => INSTAGRAM_PER_HOST_LIMIT,
        "tiktok" => TIKTOK_PER_HOST_LIMIT,
        "twitter" => TWITTER_PER_HOST_LIMIT,
        "reddit" => REDDIT_PER_HOST_LIMIT,
        _ => DEFAULT_PER_HOST_LIMIT,
    }
}

fn interval_ms_for_host(key: &str) -> u64 {
    match key {
        "youtube" => YOUTUBE_REQUEST_INTERVAL_MS,
        "instagram" => INSTAGRAM_REQUEST_INTERVAL_MS,
        "tiktok" => TIKTOK_REQUEST_INTERVAL_MS,
        "twitter" => TWITTER_REQUEST_INTERVAL_MS,
        "reddit" => REDDIT_REQUEST_INTERVAL_MS,
        _ => DEFAULT_REQUEST_INTERVAL_MS,
    }
}

pub struct HostLease {
    _permit: OwnedSemaphorePermit,
}

pub async fn acquire(host_key: &str) -> HostLease {
    let semaphore = {
        let mut guard = state().lock().await;
        guard
            .semaphores
            .entry(host_key.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(limit_for_host(host_key))))
            .clone()
    };

    let permit = semaphore
        .acquire_owned()
        .await
        .expect("host semaphore closed unexpectedly");

    let interval_ms = interval_ms_for_host(host_key);
    if interval_ms > 0 {
        let wait = {
            let mut guard = state().lock().await;
            let now = std::time::Instant::now();
            let last = guard.last_dispatch.get(host_key).copied();
            let wait = match last {
                Some(t) => {
                    let elapsed = now.duration_since(t).as_millis() as u64;
                    interval_ms.saturating_sub(elapsed)
                }
                None => 0,
            };
            guard.last_dispatch.insert(
                host_key.to_string(),
                now + std::time::Duration::from_millis(wait),
            );
            wait
        };
        if wait > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
        }
    }

    HostLease { _permit: permit }
}
