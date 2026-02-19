use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use super::auth::TelegramSessionHandle;
use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality};
use crate::platforms::traits::PlatformDownloader;

pub struct TelegramDownloader {
    session: TelegramSessionHandle,
}

impl TelegramDownloader {
    pub fn new(session: TelegramSessionHandle) -> Self {
        Self { session }
    }

    fn parse_tme_url(url: &str) -> Option<(String, Option<i32>)> {
        let parsed = url::Url::parse(url).ok()?;
        let host = parsed.host_str()?.to_lowercase();
        if host != "t.me" && !host.ends_with("telegram.me") && !host.ends_with("telegram.org") {
            return None;
        }
        let segments: Vec<&str> = parsed
            .path_segments()?
            .filter(|s| !s.is_empty())
            .collect();

        if segments.is_empty() {
            return None;
        }

        // t.me/channel/123 or t.me/c/1234567/123
        if segments.len() >= 2 {
            if segments[0] == "c" && segments.len() >= 3 {
                // Private channel: t.me/c/{channel_id}/{message_id}
                let channel = format!("c/{}", segments[1]);
                let msg_id = segments[2].parse::<i32>().ok();
                return Some((channel, msg_id));
            }
            let username = segments[0].to_string();
            let msg_id = segments[1].parse::<i32>().ok();
            return Some((username, msg_id));
        }

        // t.me/channel (no message_id)
        let first = segments[0];
        if !["joinchat", "addstickers", "login", "share"].contains(&first) {
            return Some((first.to_string(), None));
        }

        None
    }
}

#[async_trait]
impl PlatformDownloader for TelegramDownloader {
    fn name(&self) -> &str {
        "telegram"
    }

    fn can_handle(&self, url: &str) -> bool {
        Self::parse_tme_url(url).is_some()
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let (username, msg_id) = Self::parse_tme_url(url)
            .ok_or_else(|| anyhow::anyhow!("Invalid Telegram URL"))?;

        let msg_id = msg_id
            .ok_or_else(|| anyhow::anyhow!("URL must point to a specific message (e.g., t.me/channel/123)"))?;

        let guard = self.session.lock().await;
        let client = guard
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not authenticated to Telegram"))?
            .clone();
        drop(guard);

        // Resolve channel by username
        let peer = if username.starts_with("c/") {
            // Private channel with numeric ID
            let id_str = username.strip_prefix("c/").unwrap();
            let channel_id: i64 = id_str
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid channel ID"))?;
            use grammers_client::session::defs::{PeerAuth, PeerId, PeerRef};
            let peer_ref = PeerRef {
                id: PeerId::channel(channel_id),
                auth: PeerAuth::default(),
            };
            client
                .resolve_peer(peer_ref)
                .await
                .map_err(|e| anyhow::anyhow!("Cannot resolve channel: {}", e))?
        } else {
            client
                .resolve_username(&username)
                .await
                .map_err(|e| anyhow::anyhow!("Cannot resolve username: {}", e))?
                .ok_or_else(|| anyhow::anyhow!("Channel @{} not found", username))?
        };

        let messages = client
            .get_messages_by_id(&peer, &[msg_id])
            .await
            .map_err(|e| anyhow::anyhow!("Cannot fetch message: {}", e))?;

        let message = messages
            .into_iter()
            .next()
            .flatten()
            .ok_or_else(|| anyhow::anyhow!("Message {} not found", msg_id))?;

        let media = message
            .media()
            .ok_or_else(|| anyhow::anyhow!("Message has no downloadable media"))?;

        let (file_name, _file_size, media_type, format) = match &media {
            grammers_client::types::Media::Photo(photo) => {
                let name = format!("photo_{}", photo.id());
                let size = photo.size().max(0) as u64;
                (name, size, MediaType::Photo, "jpg".to_string())
            }
            grammers_client::types::Media::Document(doc) => {
                let raw_name = doc.name().to_string();
                let name = if raw_name.is_empty() {
                    format!("file_{}", doc.id())
                } else {
                    // Strip extension for title, we'll use format field
                    raw_name
                        .rsplit_once('.')
                        .map(|(n, _)| n.to_string())
                        .unwrap_or(raw_name.clone())
                };
                let size = doc.size().max(0) as u64;
                let (mt, fmt) = match doc.mime_type() {
                    Some(m) if m.starts_with("video/") => (MediaType::Video, "mp4".to_string()),
                    Some(m) if m.starts_with("audio/") => (MediaType::Audio, "mp3".to_string()),
                    Some(m) if m.starts_with("image/") => (MediaType::Photo, "jpg".to_string()),
                    _ => {
                        let ext = raw_name
                            .rsplit_once('.')
                            .map(|(_, e)| e.to_string())
                            .unwrap_or_else(|| "bin".to_string());
                        (MediaType::Video, ext)
                    }
                };
                (name, size, mt, fmt)
            }
            _ => return Err(anyhow::anyhow!("Unsupported media type")),
        };

        let channel_name = peer.name().unwrap_or("Telegram").to_string();

        Ok(MediaInfo {
            title: file_name,
            author: channel_name,
            platform: "telegram".to_string(),
            duration_seconds: None,
            thumbnail_url: None,
            available_qualities: vec![VideoQuality {
                label: "original".to_string(),
                width: 0,
                height: 0,
                url: format!("tg://{}:{}", username, msg_id),
                format,
            }],
            media_type,
            file_size_bytes: None,
        })
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<DownloadResult> {
        let _t = std::time::Instant::now();
        let quality = info
            .available_qualities
            .first()
            .ok_or_else(|| anyhow::anyhow!("No quality available"))?;

        tracing::info!("[tg-diag] download: quality.url raw={}", quality.url);
        let tg_ref = quality
            .url
            .strip_prefix("tg://")
            .ok_or_else(|| {
                tracing::error!("[tg-diag] download: quality.url missing tg:// prefix: {}", quality.url);
                anyhow::anyhow!("Invalid internal reference")
            })?;
        let (username, msg_id_str) = tg_ref
            .rsplit_once(':')
            .ok_or_else(|| {
                tracing::error!("[tg-diag] download: invalid tg ref format (no colon): {}", tg_ref);
                anyhow::anyhow!("Invalid internal reference format")
            })?;
        let msg_id: i32 = msg_id_str.parse()?;
        tracing::info!("[tg-perf] TelegramDownloader::download: parsed tg ref username={}, msg_id={}", username, msg_id);

        let guard = self.session.lock().await;
        let client = guard
            .client
            .as_ref()
            .ok_or_else(|| {
                tracing::error!("[tg-diag] download: client is None (not authenticated)");
                anyhow::anyhow!("Not authenticated to Telegram")
            })?
            .clone();
        drop(guard);

        let is_auth = client.is_authorized().await.unwrap_or(false);
        tracing::info!("[tg-diag] download: is_authorized={}", is_auth);
        if !is_auth {
            tracing::error!("[tg-diag] download: client not authorized, download will likely fail");
        }

        let peer = if username.starts_with("c/") {
            tracing::info!("[tg-diag] download: resolving private channel (c/ branch)");
            let id_str = username.strip_prefix("c/").unwrap();
            let channel_id: i64 = id_str.parse()?;
            use grammers_client::session::defs::{PeerAuth, PeerId, PeerRef};
            let peer_ref = PeerRef {
                id: PeerId::channel(channel_id),
                auth: PeerAuth::default(),
            };
            client
                .resolve_peer(peer_ref)
                .await
                .map_err(|e| {
                    tracing::error!("[tg-diag] download: failed to resolve channel: {}", e);
                    anyhow::anyhow!("Cannot resolve channel: {}", e)
                })?
        } else {
            tracing::info!("[tg-diag] download: resolving by username={}", username);
            client
                .resolve_username(username)
                .await
                .map_err(|e| {
                    tracing::error!("[tg-diag] download: failed to resolve username {}: {}", username, e);
                    anyhow::anyhow!("Cannot resolve username: {}", e)
                })?
                .ok_or_else(|| {
                    tracing::error!("[tg-diag] download: username {} resolved to None", username);
                    anyhow::anyhow!("Channel not found")
                })?
        };
        tracing::info!("[tg-perf] TelegramDownloader::download: peer resolved successfully");

        let messages = client
            .get_messages_by_id(&peer, &[msg_id])
            .await
            .map_err(|e| {
                tracing::error!("[tg-diag] download: get_messages_by_id failed: {}", e);
                anyhow::anyhow!("{}", e)
            })?;
        tracing::info!("[tg-diag] download: get_messages_by_id returned {} results", messages.len());

        let message = messages
            .into_iter()
            .next()
            .flatten()
            .ok_or_else(|| {
                tracing::error!("[tg-diag] download: message {} not found in results", msg_id);
                anyhow::anyhow!("Message not found")
            })?;
        tracing::info!("[tg-perf] TelegramDownloader::download: message found");

        let media = message
            .media()
            .ok_or_else(|| {
                tracing::error!("[tg-diag] download: message {} has no media attachment", msg_id);
                anyhow::anyhow!("No media in message")
            })?;
        let media_type_name = match &media {
            grammers_client::types::Media::Photo(_) => "Photo",
            grammers_client::types::Media::Document(_) => "Document",
            _ => "Other",
        };
        tracing::info!("[tg-diag] download: media type={}, msg_id={}", media_type_name, msg_id);

        let total_size = match &media {
            grammers_client::types::Media::Document(doc) => doc.size().max(0) as u64,
            grammers_client::types::Media::Photo(photo) => photo.size().max(0) as u64,
            _ => 0,
        };

        let filename = format!(
            "{}.{}",
            sanitize_filename::sanitize(&info.title),
            quality.format
        );
        let output_path = opts.output_dir.join(&filename);

        tokio::fs::create_dir_all(&opts.output_dir).await?;

        let mut file = tokio::fs::File::create(&output_path).await?;
        let mut download = client.iter_download(&media);
        let mut downloaded: u64 = 0;
        let mut entered_loop = false;

        loop {
            let chunk = match download.next().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("FILE_REFERENCE_EXPIRED") {
                        tracing::error!("[tg-diag] download: FILE_REFERENCE_EXPIRED — file reference has expired, message media needs re-fetch");
                    }
                    tracing::error!("[tg-diag] download: iter_download error after {} bytes: {}", downloaded, err_str);
                    return Err(anyhow::anyhow!("{}", err_str));
                }
            };
            if !entered_loop {
                tracing::info!("[tg-diag] download: first chunk received, {} bytes", chunk.len());
                entered_loop = true;
            }
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;

            if total_size > 0 {
                let percent = (downloaded as f64 / total_size as f64) * 100.0;
                let _ = progress.send(percent.min(100.0)).await;
            }
        }
        if !entered_loop {
            tracing::warn!("[tg-diag] download: download loop never entered (0 chunks received)");
        }

        file.flush().await?;
        let _ = progress.send(100.0).await;
        tracing::info!("[tg-perf] TelegramDownloader::download completed in {:?}, {} bytes", _t.elapsed(), downloaded);

        Ok(DownloadResult {
            file_path: output_path,
            file_size_bytes: downloaded,
            duration_seconds: 0.0,
        })
    }
}
