use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::core::queue::{self, emit_queue_state_from_state};
use crate::platforms::Platform;
use crate::storage::config;
use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalUrlEvent {
    pub url: String,
    pub source: String,
    pub action: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueUrlOutcome {
    Queued,
    AlreadyQueued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalUrlAction {
    Queued,
    Prefill,
    AlreadyQueued,
}

impl ExternalUrlAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Prefill => "prefill",
            Self::AlreadyQueued => "already-queued",
        }
    }
}

pub fn is_external_url(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }

    let normalized = normalize_external_url(trimmed);
    let target: &str = normalized.as_deref().unwrap_or(trimmed);

    if target.starts_with("magnet:") || target.starts_with("p2p:") {
        return true;
    }

    url::Url::parse(target)
        .map(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or(false)
}

/// Returns true when the request originated from the `omniget://` protocol
/// handler — either via the deep-link event or because the raw URL still
/// carries the scheme prefix when handed to the second-instance callback.
///
/// This is used to decide whether to bring the app window to the front:
/// the native-messaging path uses metadata written by the host process to
/// honour the extension's "Open app" toggle, but the scheme path bypasses
/// the native host entirely, so we treat scheme arrival as the user's
/// explicit intent.
pub fn arrived_via_scheme(raw_url: &str, source: &str) -> bool {
    raw_url.trim_start().starts_with("omniget:") || source == "deep-link"
}

pub fn normalize_external_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let rest = trimmed
        .strip_prefix("omniget://")
        .or_else(|| trimmed.strip_prefix("omniget:"))?;
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        return None;
    }
    if rest.starts_with("magnet:") || rest.starts_with("p2p:") {
        return Some(rest.to_string());
    }
    if rest.starts_with("http://") || rest.starts_with("https://") {
        return Some(rest.to_string());
    }
    Some(format!("https://{}", rest))
}

pub async fn queue_url_with_defaults(
    app: &AppHandle,
    url: String,
    from_hotkey: bool,
    download_mode: Option<String>,
) -> Result<QueueUrlOutcome, String> {
    let state = app.state::<AppState>();
    let settings = config::load_settings(app);
    let download_queue = state.download_queue.clone();

    {
        let mut q = download_queue.lock().await;
        q.max_concurrent = settings.advanced.max_concurrent_downloads.max(1);
        q.stagger_delay_ms = settings.advanced.stagger_delay_ms;
        if q.has_url(&url) {
            return Ok(QueueUrlOutcome::AlreadyQueued);
        }
    }

    let downloader = state
        .registry
        .find_platform(&url)
        .ok_or_else(|| "No downloader available for this URL".to_string())?;

    let platform = Platform::from_url(&url);
    let platform_name = platform
        .map(|p| p.to_string())
        .unwrap_or_else(|| "generic".to_string());

    let mut download_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let ytdlp_path = crate::core::ytdlp::find_ytdlp_cached().await;

    let ext_meta = crate::extension_storage::read_extension_metadata(&url);

    let has_ext_media = ext_meta
        .as_ref()
        .and_then(|m| m.media_type.as_deref())
        .is_some();

    if !has_ext_media && (platform_name == "youtube" || platform_name == "generic") {
        let url_clone = url.clone();
        let downloader_clone = downloader.clone();
        let platform_clone = platform_name.clone();
        let ytdlp_clone = ytdlp_path.clone();
        tokio::spawn(async move {
            queue::prefetch_info(
                &url_clone,
                &*downloader_clone,
                &platform_clone,
                ytdlp_clone.as_deref(),
            )
            .await;
        });
    }

    let output_dir = settings
        .download
        .default_output_dir
        .to_string_lossy()
        .to_string();
    let ext_referer = ext_meta.as_ref().and_then(|m| m.referer.clone());
    let ext_headers = ext_meta.as_ref().and_then(|m| m.headers.clone());
    let ext_page_url = ext_meta.as_ref().and_then(|m| m.page_url.clone());
    let ext_user_agent = ext_meta.as_ref().and_then(|m| m.user_agent.clone());

    let ext_media_info = ext_meta.as_ref().and_then(|m| {
        let mt = m.media_type.as_deref()?;
        let ct = m.content_type.as_deref().unwrap_or("");
        let format = if mt == "hls" || ct.contains("mpegurl") {
            "hls"
        } else if mt == "video" || ct.contains("video/") {
            "direct_video"
        } else if mt == "audio" || ct.contains("audio/") {
            "direct_audio"
        } else {
            return None;
        };
        let title = m
            .title
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| sanitize_filename::sanitize(s))
            .or_else(|| {
                url::Url::parse(&url).ok().and_then(|u| {
                    let path = u.path();
                    let last = path.rsplit('/').next()?;
                    if last.is_empty() {
                        return None;
                    }
                    Some(sanitize_filename::sanitize(
                        &urlencoding::decode(last)
                            .unwrap_or_else(|_| last.into())
                            .to_string(),
                    ))
                })
            })
            .unwrap_or_else(|| "download".to_string());

        Some(crate::models::media::MediaInfo {
            title,
            author: String::new(),
            platform: "generic".to_string(),
            duration_seconds: None,
            thumbnail_url: m.thumbnail.clone(),
            available_qualities: vec![crate::models::media::VideoQuality {
                label: "original".to_string(),
                width: 0,
                height: 0,
                url: url.clone(),
                format: format.to_string(),
            }],
            media_type: if format == "direct_audio" {
                crate::models::media::MediaType::Audio
            } else {
                crate::models::media::MediaType::Video
            },
            file_size_bytes: None,
        })
    });

    let ext_title = ext_meta
        .as_ref()
        .and_then(|m| m.title.as_deref())
        .filter(|s| !s.is_empty())
        .map(String::from);
    let ext_thumbnail = ext_meta.as_ref().and_then(|m| m.thumbnail.clone());

    let preview_media_info = ext_media_info.or_else(|| {
        if ext_title.is_none() && ext_thumbnail.is_none() {
            return None;
        }
        Some(crate::models::media::MediaInfo {
            title: ext_title.clone().unwrap_or_else(|| url.clone()),
            author: String::new(),
            platform: "generic".to_string(),
            duration_seconds: None,
            thumbnail_url: ext_thumbnail.clone(),
            available_qualities: Vec::new(),
            media_type: crate::models::media::MediaType::Video,
            file_size_bytes: None,
        })
    });

    let queue_title = ext_title.clone().unwrap_or_else(|| url.clone());

    {
        let mut q = download_queue.lock().await;
        download_id = q.next_available_id(download_id);
        q.enqueue(
            download_id,
            url.clone(),
            platform_name,
            queue_title,
            output_dir,
            download_mode,
            None,
            None,
            ext_referer,
            ext_headers,
            ext_page_url,
            ext_user_agent,
            preview_media_info,
            None,
            None,
            downloader,
            ytdlp_path,
            from_hotkey,
            None,
            None,
            None,
            None,
            None,
        );

        let next_ids = q.next_queued_ids();
        for nid in &next_ids {
            q.mark_active(*nid);
        }
        let state = q.get_state();
        drop(q);
        emit_queue_state_from_state(app, state);
    }

    let q_clone = download_queue.clone();
    let app_clone = app.clone();
    tokio::spawn(async move {
        let ids_to_start = {
            let q = q_clone.lock().await;
            q.items
                .iter()
                .filter(|i| i.status == queue::QueueStatus::Active)
                .filter(|i| i.id == download_id)
                .map(|i| i.id)
                .collect::<Vec<_>>()
        };

        let stagger = {
            let q = q_clone.lock().await;
            q.stagger_delay_ms
        };

        for (i, nid) in ids_to_start.into_iter().enumerate() {
            if i > 0 && stagger > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(stagger)).await;
            }
            let app_handle = app_clone.clone();
            let queue_handle = q_clone.clone();
            tokio::spawn(async move {
                queue::spawn_download(app_handle, queue_handle, nid).await;
            });
        }
    });

    Ok(QueueUrlOutcome::Queued)
}

pub async fn handle_external_url(
    app: &AppHandle,
    url: String,
    source: &str,
) -> Result<ExternalUrlAction, String> {
    let scheme_arrival = arrived_via_scheme(&url, source);
    let url = normalize_external_url(&url).unwrap_or(url);
    if !is_external_url(&url) {
        return Err("Invalid external URL".to_string());
    }

    let settings = config::load_settings(app);
    let can_queue_directly = (!settings.download.always_ask_path
        || settings.download.auto_download_on_paste)
        && has_valid_output_dir(&settings.download.default_output_dir);

    let open_app_flag = crate::extension_storage::peek_extension_open_app(&url);

    let action = if can_queue_directly {
        let outcome = queue_url_with_defaults(app, url.clone(), false, None).await?;
        if open_app_flag == Some(true) || scheme_arrival {
            crate::tray::show_window(app);
        }
        match outcome {
            QueueUrlOutcome::Queued => ExternalUrlAction::Queued,
            QueueUrlOutcome::AlreadyQueued => ExternalUrlAction::AlreadyQueued,
        }
    } else {
        crate::tray::show_window(app);
        ExternalUrlAction::Prefill
    };

    if action != ExternalUrlAction::AlreadyQueued {
        push_or_emit_event(
            app,
            ExternalUrlEvent {
                url,
                source: source.to_string(),
                action: action.as_str().to_string(),
            },
        )
        .await;
    }

    Ok(action)
}

pub async fn register_frontend(app: &AppHandle) -> Vec<ExternalUrlEvent> {
    let state = app.state::<AppState>();

    {
        let mut ready = state.frontend_ready.lock().await;
        *ready = true;
    }

    let mut pending = state.pending_external_events.lock().await;
    std::mem::take(&mut *pending)
}

pub fn find_external_url_arg<I, S>(args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    // Return the raw argument (preserving any `omniget://` prefix). The caller
    // forwards this string to `handle_external_url`, which normalises it after
    // recording whether the request arrived through the scheme — preserving
    // the prefix is what lets that detection work on Linux's single-instance
    // path, where the OS hands us the omniget:// URL as a plain CLI argument.
    args.into_iter()
        .map(|arg| arg.as_ref().trim().to_string())
        .find(|arg| is_external_url(arg))
}

async fn push_or_emit_event(app: &AppHandle, event: ExternalUrlEvent) {
    let state = app.state::<AppState>();
    let ready = {
        let ready_guard = state.frontend_ready.lock().await;
        *ready_guard
    };

    if ready {
        let _ = app.emit("external-url", &event);
    } else {
        let mut pending = state.pending_external_events.lock().await;
        pending.push(event);
    }
}

fn has_valid_output_dir(path: &PathBuf) -> bool {
    !path.as_os_str().is_empty() && path.is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_omniget_prefix_and_adds_https() {
        assert_eq!(
            normalize_external_url("omniget://www.youtube.com/watch?v=abc"),
            Some("https://www.youtube.com/watch?v=abc".to_string())
        );
    }

    #[test]
    fn normalize_preserves_explicit_http_inside_scheme_payload() {
        assert_eq!(
            normalize_external_url("omniget://https://example.com/v"),
            Some("https://example.com/v".to_string())
        );
    }

    #[test]
    fn normalize_preserves_magnet_payload() {
        assert_eq!(
            normalize_external_url("omniget:magnet:?xt=urn:btih:abc"),
            Some("magnet:?xt=urn:btih:abc".to_string())
        );
    }

    #[test]
    fn normalize_returns_none_for_non_scheme_input() {
        assert_eq!(normalize_external_url("https://example.com/v"), None);
        assert_eq!(normalize_external_url(""), None);
    }

    #[test]
    fn arrived_via_scheme_detects_prefix() {
        assert!(arrived_via_scheme(
            "omniget://www.youtube.com/watch?v=abc",
            "command-line"
        ));
        assert!(arrived_via_scheme(
            "  omniget://example.com/v",
            "command-line"
        ));
        assert!(arrived_via_scheme("omniget:magnet:?xt=foo", "command-line"));
    }

    #[test]
    fn arrived_via_scheme_detects_deep_link_source() {
        assert!(arrived_via_scheme(
            "https://www.youtube.com/watch?v=abc",
            "deep-link"
        ));
    }

    #[test]
    fn arrived_via_scheme_returns_false_for_plain_cli() {
        assert!(!arrived_via_scheme(
            "https://www.youtube.com/watch?v=abc",
            "command-line"
        ));
    }

    #[test]
    fn find_external_url_arg_preserves_omniget_prefix() {
        // The single-instance / cold-start callbacks rely on this so the
        // downstream `arrived_via_scheme` check can still detect the scheme.
        let args = vec!["--flag", "omniget://www.youtube.com/watch?v=abc"];
        assert_eq!(
            find_external_url_arg(args.iter().copied()),
            Some("omniget://www.youtube.com/watch?v=abc".to_string())
        );
    }

    #[test]
    fn find_external_url_arg_returns_https_url_unchanged() {
        let args = vec!["https://example.com/video"];
        assert_eq!(
            find_external_url_arg(args.iter().copied()),
            Some("https://example.com/video".to_string())
        );
    }

    #[test]
    fn find_external_url_arg_skips_irrelevant_args() {
        let args = vec!["--start-hidden", "--config=/etc/foo"];
        assert_eq!(find_external_url_arg(args.iter().copied()), None);
    }
}
