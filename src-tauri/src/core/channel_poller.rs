use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::core::channels::{self, ChannelFollow};

const TICK_SECONDS: u64 = 60;
const PLAYLIST_END: &str = "30";

#[derive(Clone, Serialize)]
pub struct NewVideo {
    pub id: String,
    pub title: String,
    pub url: String,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn is_due(ch: &ChannelFollow, now: u64) -> bool {
    if !ch.enabled {
        return false;
    }
    match ch.last_checked_ms {
        None => true,
        Some(last) => now.saturating_sub(last) >= (ch.interval_minutes as u64) * 60_000,
    }
}

async fn poll_channel(ch: &ChannelFollow) -> Option<Vec<NewVideo>> {
    let ytdlp = crate::core::ytdlp::find_ytdlp_cached().await?;

    let playlist_end: u32 = PLAYLIST_END.parse().unwrap_or(30);
    let listing = match crate::core::ytdlp::archive_extractor_prefix(&ch.url) {
        Some(prefix) if !ch.seen_ids.is_empty() => {
            crate::core::ytdlp::get_playlist_info_incremental(
                &ytdlp,
                &ch.url,
                &ch.seen_ids,
                prefix,
                playlist_end,
            )
            .await
        }
        _ => {
            let extra = vec!["--playlist-end".to_string(), PLAYLIST_END.to_string()];
            crate::core::ytdlp::get_playlist_info(&ytdlp, &ch.url, &extra).await
        }
    };

    match listing {
        Ok((_title, entries)) => {
            let fetched_ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
            let new_ids = channels::record_poll(&ch.id, &fetched_ids);
            if new_ids.is_empty() {
                return Some(Vec::new());
            }
            let new_set: std::collections::HashSet<&String> = new_ids.iter().collect();
            let videos = entries
                .into_iter()
                .filter(|e| new_set.contains(&e.id))
                .map(|e| NewVideo {
                    id: e.id,
                    title: e.title,
                    url: e.url,
                })
                .collect();
            Some(videos)
        }
        Err(e) => {
            tracing::warn!("[channels] poll failed for {}: {}", ch.id, e);
            channels::record_poll(&ch.id, &[]);
            None
        }
    }
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        loop {
            let now = now_ms();
            let due: Vec<ChannelFollow> = channels::list()
                .into_iter()
                .filter(|c| is_due(c, now))
                .collect();

            for ch in due {
                if let Some(videos) = poll_channel(&ch).await {
                    handle_new_videos(&app, &ch, videos).await;
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }

            tokio::time::sleep(std::time::Duration::from_secs(TICK_SECONDS)).await;
        }
    });
}

async fn handle_new_videos(app: &AppHandle, ch: &ChannelFollow, videos: Vec<NewVideo>) {
    if videos.is_empty() {
        return;
    }
    tracing::info!("[channels] {} new video(s) in {}", videos.len(), ch.id);

    if ch.auto_download {
        for v in &videos {
            if let Err(e) =
                crate::external_url::queue_url_with_defaults(app, v.url.clone(), false, None).await
            {
                tracing::warn!("[channels] auto-download enqueue failed: {}", e);
            }
        }
    }

    let _ = app
        .notification()
        .builder()
        .title(ch.title.clone())
        .body(format!("{} new video(s)", videos.len()))
        .show();
}

pub async fn check_now(app: &AppHandle, id: &str) -> Result<usize, String> {
    let ch = channels::get(id).ok_or_else(|| "Channel not found".to_string())?;
    match poll_channel(&ch).await {
        Some(videos) => {
            let n = videos.len();
            handle_new_videos(app, &ch, videos).await;
            Ok(n)
        }
        None => Err("Channel check failed".to_string()),
    }
}
