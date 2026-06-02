use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::SystemTime;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use omniget_core::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality,
};
use omniget_core::models::progress::ProgressUpdate;
use omniget_core::platforms::traits::PlatformDownloader;

const GALLERY_HOSTS: &[&str] = &[
    "deviantart.com",
    "pixiv.net",
    "artstation.com",
    "flickr.com",
    "tumblr.com",
    "danbooru.donmai.us",
    "gelbooru.com",
    "e621.net",
    "rule34.xxx",
    "konachan.com",
    "yande.re",
    "newgrounds.com",
    "kemono.su",
    "kemono.party",
    "coomer.su",
    "fanbox.cc",
    "weibo.com",
    "boards.4chan.org",
    "boards.4channel.org",
    "imgur.com",
    "imgchest.com",
    "nhentai.net",
    "hentai-foundry.com",
    "baraag.net",
    "wallhaven.cc",
    "subscribestar.adult",
];

pub fn is_gallery_url(url: &str) -> bool {
    let host = match url::Url::parse(url) {
        Ok(u) => u
            .host_str()
            .map(|h| h.trim_start_matches("www.").to_lowercase()),
        Err(_) => None,
    };
    let Some(host) = host else {
        return false;
    };
    GALLERY_HOSTS
        .iter()
        .any(|h| host == *h || host.ends_with(&format!(".{}", h)))
}

pub struct GalleryDlDownloader;

impl GalleryDlDownloader {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GalleryDlDownloader {
    fn default() -> Self {
        Self::new()
    }
}

fn title_from_url(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        let host = parsed
            .host_str()
            .unwrap_or("gallery")
            .trim_start_matches("www.")
            .to_string();
        let last = parsed
            .path_segments()
            .and_then(|s| s.filter(|p| !p.is_empty()).last())
            .unwrap_or("")
            .to_string();
        if last.is_empty() {
            host
        } else {
            format!("{} - {}", host, last)
        }
    } else {
        "gallery".to_string()
    }
}

#[async_trait]
impl PlatformDownloader for GalleryDlDownloader {
    fn name(&self) -> &str {
        "gallery"
    }

    fn can_handle(&self, url: &str) -> bool {
        is_gallery_url(url)
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let title = title_from_url(url);
        Ok(MediaInfo {
            title,
            author: String::new(),
            platform: "gallery".to_string(),
            duration_seconds: None,
            thumbnail_url: None,
            available_qualities: vec![VideoQuality {
                label: "Gallery".to_string(),
                width: 0,
                height: 0,
                url: url.to_string(),
                format: "gallery".to_string(),
            }],
            media_type: MediaType::Carousel,
            file_size_bytes: None,
        })
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<DownloadResult> {
        let url = info
            .available_qualities
            .first()
            .map(|q| q.url.clone())
            .ok_or_else(|| anyhow!("No gallery URL available"))?;

        let bin = omniget_core::core::dependencies::ensure_gallerydl()
            .await
            .ok_or_else(|| {
                anyhow!(
                    "gallery-dl is not available. Install gallery-dl to download from this site."
                )
            })?;

        std::fs::create_dir_all(&opts.output_dir)?;
        let _ = progress.send(ProgressUpdate::percent(-2.0)).await;

        let mut cmd = omniget_core::core::process::command(&bin);
        cmd.arg("--no-colors")
            .arg("-D")
            .arg(&opts.output_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(data_dir) = omniget_core::core::paths::app_data_dir() {
            let archive_dir = data_dir.join("archive");
            if std::fs::create_dir_all(&archive_dir).is_ok() {
                cmd.arg("--download-archive")
                    .arg(archive_dir.join("gallery-dl.txt"));
            }
            let cookie_file = data_dir.join("chrome-extension-cookies.txt");
            if std::fs::metadata(&cookie_file)
                .map(|m| m.len() > 0)
                .unwrap_or(false)
            {
                cmd.arg("--cookies").arg(&cookie_file);
            }
        }

        cmd.arg("--").arg(&url);

        cmd.kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("Failed to start gallery-dl: {}", e))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("No stdout from gallery-dl"))?;
        let stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("No stderr from gallery-dl"))?;

        let started_at = SystemTime::now();

        let progress_tx = progress.clone();
        let reader_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut count: u64 = 0;
            let mut last_emit = std::time::Instant::now();
            while let Ok(Some(line)) = lines.next_line().await {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('[') {
                    continue;
                }
                count += 1;
                if last_emit.elapsed() >= std::time::Duration::from_millis(250) {
                    let pct = ((count as f64 / (count as f64 + 4.0)) * 100.0).min(95.0);
                    let _ = progress_tx.send(ProgressUpdate::percent(pct)).await;
                    last_emit = std::time::Instant::now();
                }
            }
            count
        });

        let stderr_task = tokio::spawn(async move {
            let mut buf = String::new();
            let mut lines = BufReader::new(stderr_pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                buf.push_str(&line);
                buf.push('\n');
            }
            buf
        });

        let status = tokio::select! {
            s = child.wait() => s.map_err(|e| anyhow!("gallery-dl process failed: {}", e))?,
            _ = opts.cancel_token.cancelled() => {
                let _ = child.kill().await;
                let _ = reader_task.await;
                let _ = stderr_task.await;
                anyhow::bail!("Download cancelled");
            }
        };

        let _ = reader_task.await;
        let stderr_content = stderr_task.await.unwrap_or_default();

        if !status.success() {
            let detail = stderr_content
                .lines()
                .rev()
                .find(|l| {
                    let l = l.to_lowercase();
                    l.contains("error")
                        || l.contains("forbidden")
                        || l.contains("not found")
                        || l.contains("unsupported")
                })
                .unwrap_or("gallery-dl failed")
                .trim()
                .to_string();
            return Err(anyhow!(detail));
        }

        let (file_path, total_bytes) = collect_new_files(&opts.output_dir, started_at);
        if total_bytes == 0 {
            return Err(anyhow!(
                "gallery-dl completed but downloaded no new files (already archived or empty gallery)"
            ));
        }

        let _ = progress.send(ProgressUpdate::percent(100.0)).await;

        Ok(DownloadResult {
            file_path,
            file_size_bytes: total_bytes,
            duration_seconds: 0.0,
            torrent_id: None,
        })
    }
}

fn collect_new_files(dir: &Path, since: SystemTime) -> (PathBuf, u64) {
    let mut total: u64 = 0;
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    let mut new_count = 0usize;
    let mut single: Option<PathBuf> = None;

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if modified < since {
                continue;
            }
            total += meta.len();
            new_count += 1;
            single = Some(path.clone());
            if newest.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
                newest = Some((modified, path));
            }
        }
    }

    let file_path = if new_count == 1 {
        single.unwrap_or_else(|| dir.to_path_buf())
    } else {
        dir.to_path_buf()
    };
    (file_path, total)
}

#[cfg(test)]
mod tests {
    use super::is_gallery_url;

    #[test]
    fn matches_curated_gallery_hosts() {
        assert!(is_gallery_url(
            "https://www.deviantart.com/someartist/gallery"
        ));
        assert!(is_gallery_url("https://www.pixiv.net/en/users/12345"));
        assert!(is_gallery_url("https://danbooru.donmai.us/posts?tags=foo"));
        assert!(is_gallery_url("https://imgur.com/a/abcd"));
        assert!(is_gallery_url("https://blog.tumblr.com/"));
    }

    #[test]
    fn ignores_non_gallery_hosts() {
        assert!(!is_gallery_url("https://www.youtube.com/watch?v=x"));
        assert!(!is_gallery_url("https://www.instagram.com/p/abc/"));
        assert!(!is_gallery_url("https://example.com/image.jpg"));
        assert!(!is_gallery_url("not a url"));
    }
}
