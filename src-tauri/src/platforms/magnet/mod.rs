use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use librqbit::{Session, SessionOptions, AddTorrent, AddTorrentOptions, api::TorrentIdOrHash};

use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality};
use crate::platforms::traits::PlatformDownloader;

type SharedSession = Arc<tokio::sync::Mutex<Option<Arc<Session>>>>;

pub struct MagnetDownloader {
    session: SharedSession,
    session_output_dir: Arc<tokio::sync::Mutex<Option<PathBuf>>>,
}

impl MagnetDownloader {
    pub fn new(session: SharedSession) -> Self {
        Self {
            session,
            session_output_dir: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }
}

#[async_trait]
impl PlatformDownloader for MagnetDownloader {
    fn name(&self) -> &str {
        "magnet"
    }

    fn can_handle(&self, url: &str) -> bool {
        url.starts_with("magnet:") || url.ends_with(".torrent")
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let mut title = "Torrent Download".to_string();

        if url.starts_with("magnet:") {
            if let Ok(parsed_url) = url::Url::parse(url) {
                for (key, value) in parsed_url.query_pairs() {
                    if key == "dn" {
                        title = value.to_string();
                        break;
                    }
                }
            }
        }

        Ok(MediaInfo {
            title,
            author: "BitTorrent".to_string(),
            platform: "magnet".to_string(),
            duration_seconds: None,
            thumbnail_url: None,
            available_qualities: vec![VideoQuality {
                label: "Original".to_string(),
                width: 0,
                height: 0,
                url: url.to_string(),
                format: "torrent".to_string(),
            }],
            media_type: MediaType::Video,
            file_size_bytes: None,
        })
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<DownloadResult> {
        let _ = progress.send(0.0).await;

        let url = match info.available_qualities.first() {
            Some(q) => &q.url,
            None => anyhow::bail!("No URL found in MediaInfo"),
        };

        let output_dir = &opts.output_dir;

        // Get or create the shared session, recreating if output_dir changed
        let session = {
            let mut guard = self.session.lock().await;
            let mut dir_guard = self.session_output_dir.lock().await;
            let need_new_session = match (&*guard, &*dir_guard) {
                (Some(_), Some(prev_dir)) => prev_dir != output_dir,
                (None, _) => true,
                _ => true,
            };
            if need_new_session {
                if guard.is_some() {
                    tracing::info!("[magnet] output dir changed, recreating session");
                }
                let listen_port = opts.torrent_listen_port.unwrap_or(6881).min(65525);
                tracing::info!("[magnet] creating shared session, port: {}, output: {}", listen_port, output_dir.display());
                let session_opts = SessionOptions {
                    listen_port_range: Some(listen_port..listen_port.saturating_add(10)),
                    ..Default::default()
                };
                let cancel_rx = opts.cancel_token.clone();
                let s = tokio::select! {
                    result = Session::new_with_opts(output_dir.into(), session_opts) => {
                        result.map_err(|e| anyhow::anyhow!("Failed to initialize torrent session: {}", e))?
                    }
                    _ = cancel_rx.cancelled() => {
                        anyhow::bail!("Download cancelled during session creation");
                    }
                };
                *guard = Some(s.clone());
                *dir_guard = Some(output_dir.clone());
                s
            } else {
                tracing::info!("[magnet] reusing existing session");
                guard.as_ref().unwrap().clone()
            }
        };

        let add_torrent = AddTorrent::from_url(url);
        let torrent_opts = AddTorrentOptions {
            overwrite: true,
            ..Default::default()
        };

        tracing::info!("[magnet] adding torrent, output: {}", output_dir.display());
        let (torrent_id, managed_torrent) = match session.add_torrent(add_torrent, Some(torrent_opts)).await {
            Ok(resp) => match resp {
                librqbit::AddTorrentResponse::Added(id, handle) => (id, handle),
                librqbit::AddTorrentResponse::AlreadyManaged(id, handle) => (id, handle),
                librqbit::AddTorrentResponse::ListOnly(_) => {
                    anyhow::bail!("Torrent was added in list-only mode");
                }
            },
            Err(e) => anyhow::bail!("Failed to add torrent: {}", e),
        };

        tracing::info!("[magnet] torrent added (id={}), waiting for download...", torrent_id);

        if let Some(slot) = &opts.torrent_id_slot {
            *slot.lock().await = Some(torrent_id);
        }

        let completion = managed_torrent.wait_until_completed();
        tokio::pin!(completion);

        let cancel_rx = opts.cancel_token.clone();
        let session_for_cancel = session.clone();

        loop {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                    if managed_torrent.is_paused() {
                        continue;
                    }
                    let stats = managed_torrent.stats();
                    let total = stats.total_bytes;
                    let downloaded = stats.progress_bytes;

                    if total > 0 {
                        let pct = (downloaded as f64 / total as f64) * 100.0;
                        let _ = progress.send(pct).await;
                        tracing::debug!(
                            "[magnet] progress: {:.1}% ({:.1} MB / {:.1} MB)",
                            pct,
                            downloaded as f64 / 1_048_576.0,
                            total as f64 / 1_048_576.0,
                        );
                        // Fallback: detect completion from stats when
                        // wait_until_completed() doesn't resolve
                        if downloaded >= total {
                            tracing::info!("[magnet] download complete from stats (id={})", torrent_id);
                            break;
                        }
                    }
                }
                _ = cancel_rx.cancelled() => {
                    tracing::info!("[magnet] download cancelled, removing torrent id={}", torrent_id);
                    if let Err(e) = session_for_cancel.delete(TorrentIdOrHash::Id(torrent_id), false).await {
                        tracing::warn!("[magnet] failed to delete torrent on cancel: {}", e);
                    }
                    anyhow::bail!("Download cancelled");
                }
                res = &mut completion => {
                    if let Err(e) = res {
                        anyhow::bail!("Torrent download failed: {}", e);
                    }
                    let _ = progress.send(100.0).await;
                    tracing::info!("[magnet] download complete (id={})", torrent_id);
                    break;
                }
            }
        }

        let (total_size, torrent_name) = managed_torrent.with_metadata(|meta| {
            let size = meta.info.iter_file_lengths().ok().map(|iter| iter.sum::<u64>())
                .unwrap_or_else(|| meta.file_infos.iter().map(|f| f.len).sum());
            let name = meta.info.name.as_ref()
                .map(|n| String::from_utf8_lossy(n.as_ref()).to_string())
                .unwrap_or_default();
            (size, name)
        }).unwrap_or((0, String::new()));

        let file_path = if torrent_name.is_empty() {
            output_dir.clone()
        } else {
            output_dir.join(&torrent_name)
        };

        Ok(DownloadResult {
            file_path,
            file_size_bytes: total_size,
            duration_seconds: 0.0,
            torrent_id: Some(torrent_id),
        })
    }
}
