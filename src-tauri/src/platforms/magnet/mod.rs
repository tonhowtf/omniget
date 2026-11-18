use omniget_core::models::progress::ProgressUpdate;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use librqbit::{api::TorrentIdOrHash, AddTorrent, AddTorrentOptions, Session, SessionOptions};
use tokio::sync::mpsc;

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

fn folder_for_torrent(session_dir: &Path, listed_dir: &Path, output_dir: &Path) -> PathBuf {
    match listed_dir.strip_prefix(session_dir) {
        Ok(sub_folder) if !sub_folder.as_os_str().is_empty() => output_dir.join(sub_folder),
        _ => output_dir.to_path_buf(),
    }
}

#[async_trait]
impl PlatformDownloader for MagnetDownloader {
    fn name(&self) -> &str {
        "magnet"
    }

    fn can_handle(&self, url: &str) -> bool {
        url.starts_with("magnet:")
            || url.ends_with(".torrent")
            || (std::path::Path::new(url).exists()
                && std::path::Path::new(url)
                    .extension()
                    .map(|e| e == "torrent")
                    .unwrap_or(false))
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
        } else {
            let path = std::path::Path::new(url);
            if path.exists() {
                if let Some(stem) = path.file_stem() {
                    title = stem.to_string_lossy().to_string();
                }
            } else if url.ends_with(".torrent") {
                if let Some(name) = url.rsplit('/').next() {
                    title = name.trim_end_matches(".torrent").to_string();
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
        progress: mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<DownloadResult> {
        let _ = progress.send(ProgressUpdate::percent(0.0)).await;

        let url = match info.available_qualities.first() {
            Some(q) => &q.url,
            None => anyhow::bail!("No URL found in MediaInfo"),
        };

        let output_dir = &opts.output_dir;

        let (session, session_dir) = {
            let mut guard = self.session.lock().await;
            let mut dir_guard = self.session_output_dir.lock().await;
            let existing = match (&*guard, &*dir_guard) {
                (Some(session), Some(dir)) => Some((session.clone(), dir.clone())),
                _ => None,
            };
            if let Some(existing) = existing {
                tracing::info!("[magnet] reusing existing session");
                existing
            } else {
                let listen_port = opts.torrent_listen_port.unwrap_or(6881).min(65525);
                tracing::info!(
                    "[magnet] creating shared session, port: {}, output: {}",
                    listen_port,
                    output_dir.display()
                );
                let enable_upnp = opts.torrent_upnp;
                let make_opts = |disable_dht_persistence: bool, disable_dht: bool| SessionOptions {
                    listen_port_range: Some(listen_port..listen_port.saturating_add(10)),
                    disable_dht,
                    disable_dht_persistence,
                    enable_upnp_port_forwarding: enable_upnp,
                    ..Default::default()
                };
                let cancel_rx = opts.cancel_token.clone();
                let output_pb: PathBuf = output_dir.into();
                let s = tokio::select! {
                    result = async {
                        match Session::new_with_opts(output_pb.clone(), make_opts(false, false)).await {
                            Ok(s) => Ok(s),
                            Err(e) => {
                                tracing::warn!(
                                    "[magnet] persistent DHT init failed ({}); retrying without DHT persistence",
                                    e
                                );
                                match Session::new_with_opts(output_pb.clone(), make_opts(true, false)).await {
                                    Ok(s) => Ok(s),
                                    Err(e2) => {
                                        tracing::warn!(
                                            "[magnet] DHT init failed ({}); retrying with DHT disabled (trackers/PEX only)",
                                            e2
                                        );
                                        Session::new_with_opts(output_pb, make_opts(true, true)).await
                                    }
                                }
                            }
                        }
                    } => {
                        result.map_err(|e| anyhow::anyhow!("Failed to initialize torrent session: {}", e))?
                    }
                    _ = cancel_rx.cancelled() => {
                        anyhow::bail!("Download cancelled during session creation");
                    }
                };
                *guard = Some(s.clone());
                *dir_guard = Some(output_dir.clone());
                (s, output_dir.clone())
            }
        };

        let add_torrent = if url.starts_with("magnet:")
            || url.starts_with("http://")
            || url.starts_with("https://")
        {
            AddTorrent::from_url(url)
        } else {
            let path = std::path::Path::new(url);
            if path.exists() && path.extension().map(|e| e == "torrent").unwrap_or(false) {
                let bytes = tokio::fs::read(path)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to read .torrent file: {}", e))?;
                AddTorrent::from_bytes(bytes)
            } else {
                AddTorrent::from_url(url)
            }
        };
        let only_files = opts
            .torrent_files
            .as_ref()
            .filter(|v| !v.is_empty())
            .cloned();
        if let Some(sel) = &only_files {
            tracing::info!("[magnet] selective download: {} file(s)", sel.len());
        }
        let trackers = if opts.torrent_auto_trackers {
            let list = crate::core::trackers::extra_trackers();
            tracing::info!("[magnet] injecting {} public trackers", list.len());
            Some(list)
        } else {
            None
        };
        let mut torrent_opts = AddTorrentOptions {
            overwrite: true,
            only_files: only_files.clone(),
            trackers: trackers.clone(),
            ..Default::default()
        };
        let add_torrent = if *output_dir == session_dir {
            add_torrent
        } else {
            let list_opts = AddTorrentOptions {
                list_only: true,
                only_files,
                trackers,
                ..Default::default()
            };
            let listed = match session.add_torrent(add_torrent, Some(list_opts)).await {
                Ok(librqbit::AddTorrentResponse::ListOnly(listed)) => listed,
                Ok(_) => anyhow::bail!("Failed to read torrent metadata"),
                Err(e) => anyhow::bail!("Failed to add torrent: {}", e),
            };
            torrent_opts.output_folder = Some(
                folder_for_torrent(&session_dir, &listed.output_folder, output_dir)
                    .to_string_lossy()
                    .into_owned(),
            );
            torrent_opts.initial_peers = Some(listed.seen_peers);
            AddTorrent::from_bytes(listed.torrent_bytes)
        };

        tracing::info!("[magnet] adding torrent, output: {}", output_dir.display());
        let (torrent_id, managed_torrent) =
            match session.add_torrent(add_torrent, Some(torrent_opts)).await {
                Ok(resp) => match resp {
                    librqbit::AddTorrentResponse::Added(id, handle) => (id, handle),
                    librqbit::AddTorrentResponse::AlreadyManaged(id, handle) => (id, handle),
                    librqbit::AddTorrentResponse::ListOnly(_) => {
                        anyhow::bail!("Torrent was added in list-only mode");
                    }
                },
                Err(e) => anyhow::bail!("Failed to add torrent: {}", e),
            };

        tracing::info!(
            "[magnet] torrent added (id={}), waiting for download...",
            torrent_id
        );

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
                    let (speed, peers) = stats
                        .live
                        .as_ref()
                        .map(|live| {
                            (
                                live.download_speed.mbps * 1024.0 * 1024.0,
                                Some(live.snapshot.peer_stats.live as u32),
                            )
                        })
                        .unwrap_or((0.0, None));

                    if total == 0 {
                        let mut update = ProgressUpdate::rich(0.0, Some(downloaded), None, Some(speed), None);
                        update.peers = Some(peers.unwrap_or(0));
                        update.indeterminate = true;
                        let _ = progress.send(update).await;
                    } else {
                        let pct = (downloaded as f64 / total as f64) * 100.0;
                        let mut update = ProgressUpdate::rich(
                            pct,
                            Some(downloaded),
                            Some(total),
                            Some(speed),
                            None,
                        );
                        update.peers = peers;
                        let _ = progress.send(update).await;
                        tracing::debug!(
                            "[magnet] progress: {:.1}% ({:.1} MB / {:.1} MB)",
                            pct,
                            downloaded as f64 / 1_048_576.0,
                            total as f64 / 1_048_576.0,
                        );
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
                    let _ = progress.send(ProgressUpdate::percent(100.0)).await;
                    tracing::info!("[magnet] download complete (id={})", torrent_id);
                    break;
                }
            }
        }

        let (total_size, torrent_name) = managed_torrent
            .with_metadata(|meta| {
                let size = meta
                    .info
                    .iter_file_lengths()
                    .ok()
                    .map(|iter| iter.sum::<u64>())
                    .unwrap_or_else(|| meta.file_infos.iter().map(|f| f.len).sum());
                let name = meta
                    .info
                    .name
                    .as_ref()
                    .map(|n| String::from_utf8_lossy(n.as_ref()).to_string())
                    .unwrap_or_default();
                (size, name)
            })
            .unwrap_or((0, String::new()));

        let file_path = if torrent_name.is_empty() {
            output_dir.clone()
        } else {
            output_dir.join(&torrent_name)
        };

        let seeding = if opts.torrent_stop_seeding {
            if let Err(e) = session.delete(TorrentIdOrHash::Id(torrent_id), false).await {
                tracing::warn!("[magnet] failed to stop seeding id={}: {}", torrent_id, e);
            }
            None
        } else {
            Some(torrent_id)
        };

        Ok(DownloadResult {
            file_path,
            file_size_bytes: total_size,
            duration_seconds: 0.0,
            torrent_id: seeding,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_file_torrent_lands_in_the_folder_the_user_picked() {
        assert_eq!(
            folder_for_torrent(
                Path::new("/first"),
                Path::new("/first"),
                Path::new("/second")
            ),
            PathBuf::from("/second")
        );
    }

    #[test]
    fn a_multi_file_torrent_keeps_its_own_folder_inside_the_one_picked() {
        assert_eq!(
            folder_for_torrent(
                Path::new("/first"),
                Path::new("/first/Some Album"),
                Path::new("/second")
            ),
            Path::new("/second").join("Some Album")
        );
    }
}
