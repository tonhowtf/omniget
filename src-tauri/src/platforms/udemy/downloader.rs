use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use futures::StreamExt;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::core::media_processor::MediaProcessor;

use super::api::{self, UdemyCourse};
use super::auth::UdemySession;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36 Edg/145.0.0.0";
const MAX_FILENAME_BYTES: usize = 200;

#[derive(Debug, Clone, serde::Serialize)]
pub struct UdemyCourseDownloadProgress {
    pub course_id: u64,
    pub course_name: String,
    pub percent: f64,
    pub current_chapter: String,
    pub current_lecture: String,
    pub downloaded_bytes: u64,
    pub total_lectures: u32,
    pub completed_lectures: u32,
}

pub struct UdemyDownloader {
    session: Arc<Mutex<Option<UdemySession>>>,
    hls_client: reqwest::Client,
    max_concurrent_segments: u32,
    max_retries: u32,
}

fn safe_filename(name: &str) -> String {
    let sanitized = sanitize_filename::sanitize(name);
    if sanitized.len() <= MAX_FILENAME_BYTES {
        return sanitized;
    }
    let mut end = MAX_FILENAME_BYTES;
    while !sanitized.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    sanitized[..end].to_string()
}

impl UdemyDownloader {
    pub fn new(
        session: Arc<Mutex<Option<UdemySession>>>,
        max_concurrent_segments: u32,
        max_retries: u32,
    ) -> Self {
        let hls_client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(300))
            .build()
            .unwrap_or_default();

        Self {
            session,
            hls_client,
            max_concurrent_segments,
            max_retries,
        }
    }

    pub async fn download_full_course(
        &self,
        course: &UdemyCourse,
        output_dir: &str,
        curriculum: api::UdemyCurriculum,
        progress_tx: mpsc::Sender<UdemyCourseDownloadProgress>,
        cancel_token: CancellationToken,
    ) -> anyhow::Result<()> {
        let session = {
            let guard = self.session.lock().await;
            guard.clone().ok_or_else(|| anyhow!("Not authenticated"))?
        };

        let _ = progress_tx.send(UdemyCourseDownloadProgress {
            course_id: course.id,
            course_name: course.title.clone(),
            percent: 0.0,
            current_chapter: String::new(),
            current_lecture: String::new(),
            downloaded_bytes: 0,
            total_lectures: curriculum.total_lectures,
            completed_lectures: 0,
        }).await;

        let course_dir_name = safe_filename(&course.title);
        let course_dir = PathBuf::from(output_dir).join(&course_dir_name);
        tokio::fs::create_dir_all(&course_dir).await?;

        let total_lectures = curriculum.total_lectures;
        let mut completed_lectures: u32 = 0;
        let mut downloaded_bytes: u64 = 0;

        for (ch_idx, chapter) in curriculum.chapters.iter().enumerate() {
            if cancel_token.is_cancelled() {
                return Err(anyhow!("Download cancelled"));
            }

            let chapter_dir_name = format!(
                "{:02} - {}",
                ch_idx + 1,
                safe_filename(&chapter.title)
            );
            let chapter_dir = course_dir.join(&chapter_dir_name);
            tokio::fs::create_dir_all(&chapter_dir).await?;

            for (lec_idx, lecture) in chapter.lectures.iter().enumerate() {
                if cancel_token.is_cancelled() {
                    return Err(anyhow!("Download cancelled"));
                }

                let lecture_num = (lec_idx + 1) as u32;

                let _ = progress_tx.send(UdemyCourseDownloadProgress {
                    course_id: course.id,
                    course_name: course.title.clone(),
                    percent: if total_lectures > 0 {
                        (completed_lectures as f64 / total_lectures as f64) * 100.0
                    } else {
                        0.0
                    },
                    current_chapter: chapter.title.clone(),
                    current_lecture: format!("{}/{} - {}", lecture_num, chapter.lectures.len(), lecture.title),
                    downloaded_bytes,
                    total_lectures,
                    completed_lectures,
                }).await;

                let bytes = self.download_lecture(
                    &session,
                    lecture,
                    &chapter_dir,
                    &cancel_token,
                    lecture_num,
                ).await;

                match bytes {
                    Ok(b) => {
                        downloaded_bytes += b;
                    }
                    Err(e) => {
                        tracing::error!(
                            "[udemy] failed to download lecture '{}': {}",
                            lecture.title, e
                        );
                    }
                }

                if lecture.lecture_class == "lecture" {
                    completed_lectures += 1;
                }
            }
        }

        let _ = progress_tx.send(UdemyCourseDownloadProgress {
            course_id: course.id,
            course_name: course.title.clone(),
            percent: 100.0,
            current_chapter: String::new(),
            current_lecture: String::new(),
            downloaded_bytes,
            total_lectures,
            completed_lectures,
        }).await;

        tracing::info!(
            "[udemy] course '{}' download complete: {} lectures, {} bytes",
            course.title, completed_lectures, downloaded_bytes
        );

        Ok(())
    }

    async fn download_lecture(
        &self,
        session: &UdemySession,
        lecture: &api::UdemyLecture,
        chapter_dir: &Path,
        cancel_token: &CancellationToken,
        lecture_num: u32,
    ) -> anyhow::Result<u64> {
        if lecture.lecture_class == "quiz" || lecture.lecture_class == "practice" {
            tracing::info!("[udemy] skipping {}: {}", lecture.lecture_class, lecture.title);
            return Ok(0);
        }

        let asset = match &lecture.asset {
            Some(a) => a,
            None => return Ok(0),
        };

        let asset_type = asset.get("asset_type")
            .or_else(|| asset.get("assetType"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();

        let mut total_bytes: u64 = 0;

        match asset_type.as_str() {
            "video" => {
                total_bytes += self.download_video_asset(
                    session, asset, &lecture.title, chapter_dir, cancel_token, lecture_num
                ).await?;
            }
            "article" => {
                let body = asset.get("body").and_then(|v| v.as_str()).unwrap_or("");
                if !body.is_empty() {
                    let file_name = format!(
                        "{:02} - {}.html",
                        lecture_num,
                        safe_filename(&lecture.title)
                    );
                    let file_path = chapter_dir.join(&file_name);
                    if !file_exists_with_content(&file_path).await {
                        tokio::fs::write(&file_path, body.as_bytes()).await?;
                        tracing::info!("[udemy] saved article: {}", file_name);
                    }
                }
            }
            "file" | "e-book" | "presentation" | "audio" => {
                total_bytes += self.download_downloadable_asset(
                    session, asset, &lecture.title, chapter_dir, lecture_num
                ).await?;
            }
            _ => {
                if !asset_type.is_empty() {
                    tracing::warn!("[udemy] unknown asset type '{}' for '{}'", asset_type, lecture.title);
                }
            }
        }

        let captions = asset.get("captions").and_then(|v| v.as_array());
        if let Some(tracks) = captions {
            for track in tracks {
                let class = track.get("_class").and_then(|v| v.as_str()).unwrap_or("");
                if class != "caption" {
                    continue;
                }
                let url = match track.get("url").and_then(|v| v.as_str()) {
                    Some(u) => u,
                    None => continue,
                };
                let lang = track.get("language")
                    .or_else(|| track.get("srclang"))
                    .or_else(|| track.get("label"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                let ext = if url.contains(".vtt") { "vtt" } else { "srt" };
                let caption_name = format!(
                    "{:02} - {}_{}.{}",
                    lecture_num,
                    safe_filename(&lecture.title),
                    lang,
                    ext
                );
                let caption_path = chapter_dir.join(&caption_name);

                if !file_exists_with_content(&caption_path).await {
                    match download_file_simple(&session.client, url, &caption_path).await {
                        Ok(b) => {
                            total_bytes += b;
                            tracing::info!("[udemy] saved caption: {}", caption_name);
                        }
                        Err(e) => {
                            tracing::warn!("[udemy] failed to download caption '{}': {}", caption_name, e);
                        }
                    }
                }
            }
        }

        let supp_assets = asset.get("supplementary_assets").and_then(|v| v.as_array());
        if let Some(assets) = supp_assets {
            for supp in assets {
                total_bytes += self.download_supplementary_asset(
                    session, supp, chapter_dir, lecture_num
                ).await.unwrap_or(0);
            }
        }

        Ok(total_bytes)
    }

    async fn download_video_asset(
        &self,
        session: &UdemySession,
        asset: &serde_json::Value,
        title: &str,
        chapter_dir: &Path,
        cancel_token: &CancellationToken,
        lecture_num: u32,
    ) -> anyhow::Result<u64> {
        let file_name = format!("{:02} - {}.mp4", lecture_num, safe_filename(title));
        let file_path = chapter_dir.join(&file_name);

        if file_exists_with_content(&file_path).await {
            tracing::info!("[udemy] skipping existing video: {}", file_name);
            return Ok(0);
        }

        let stream_urls = asset.get("stream_urls");
        if let Some(streams) = stream_urls {
            if let Some(videos) = streams.get("Video").and_then(|v| v.as_array()) {
                return self.download_from_stream_urls(
                    session, videos, &file_path, title, cancel_token
                ).await;
            }
        }

        let media_sources = asset.get("media_sources");
        if media_sources.is_some() {
            tracing::warn!(
                "[udemy] DRM video skipped: '{}' (media_sources without stream_urls)",
                title
            );
            return Ok(0);
        }

        tracing::warn!("[udemy] no video sources found for '{}'", title);
        Ok(0)
    }

    async fn download_from_stream_urls(
        &self,
        session: &UdemySession,
        videos: &[serde_json::Value],
        file_path: &Path,
        title: &str,
        cancel_token: &CancellationToken,
    ) -> anyhow::Result<u64> {
        let mut sources: Vec<(&str, u32)> = Vec::new();
        for v in videos {
            let label = v.get("label").and_then(|l| l.as_str()).unwrap_or("0");
            let url = match v.get("file").and_then(|f| f.as_str()) {
                Some(u) => u,
                None => continue,
            };
            if label.to_lowercase() == "audio" {
                continue;
            }
            let height: u32 = label.parse().unwrap_or(0);
            sources.push((url, height));
        }

        if sources.is_empty() {
            tracing::warn!("[udemy] no valid video sources for '{}'", title);
            return Ok(0);
        }

        sources.sort_by(|a, b| b.1.cmp(&a.1));
        let (best_url, best_height) = sources[0];

        tracing::info!("[udemy] downloading '{}' at {}p", title, best_height);

        let url_str = best_url.to_string();
        let is_hls = url_str.contains(".m3u8");

        if is_hls {
            let output_str = file_path.to_string_lossy().to_string();
            let result = MediaProcessor::download_hls(
                &url_str,
                &output_str,
                "https://www.udemy.com/",
                None,
                cancel_token.clone(),
                self.max_concurrent_segments,
                self.max_retries,
                Some(self.hls_client.clone()),
            ).await;
            match result {
                Ok(r) => {
                    tracing::info!("[udemy] HLS download complete: {}", title);
                    Ok(r.file_size)
                }
                Err(e) => {
                    let _ = tokio::fs::remove_file(file_path).await;
                    Err(e)
                }
            }
        } else {
            let bytes = download_file_simple(&session.client, &url_str, file_path).await?;
            tracing::info!("[udemy] direct download complete: {} ({}p, {} bytes)", title, best_height, bytes);
            Ok(bytes)
        }
    }

    async fn download_downloadable_asset(
        &self,
        session: &UdemySession,
        asset: &serde_json::Value,
        title: &str,
        chapter_dir: &Path,
        lecture_num: u32,
    ) -> anyhow::Result<u64> {
        let filename = asset.get("filename")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let download_url = asset.get("download_urls")
            .and_then(|d| {
                if let Some(obj) = d.as_object() {
                    for (_key, val) in obj {
                        if let Some(arr) = val.as_array() {
                            if let Some(first) = arr.first() {
                                if let Some(url) = first.get("file").and_then(|f| f.as_str()) {
                                    return Some(url.to_string());
                                }
                            }
                        }
                    }
                }
                None
            });

        let url = match download_url {
            Some(u) => u,
            None => {
                tracing::warn!("[udemy] no download URL for asset '{}'", title);
                return Ok(0);
            }
        };

        let safe_name = if filename.is_empty() {
            format!("{:02} - {}.bin", lecture_num, safe_filename(title))
        } else {
            format!("{:02} - {}", lecture_num, safe_filename(filename))
        };

        let file_path = chapter_dir.join(&safe_name);

        if file_exists_with_content(&file_path).await {
            tracing::info!("[udemy] skipping existing file: {}", safe_name);
            return Ok(0);
        }

        match download_file_simple(&session.client, &url, &file_path).await {
            Ok(b) => {
                tracing::info!("[udemy] downloaded asset: {}", safe_name);
                Ok(b)
            }
            Err(e) => {
                tracing::error!("[udemy] failed to download '{}': {}", safe_name, e);
                Ok(0)
            }
        }
    }

    async fn download_supplementary_asset(
        &self,
        session: &UdemySession,
        supp: &serde_json::Value,
        chapter_dir: &Path,
        lecture_num: u32,
    ) -> anyhow::Result<u64> {
        let asset_type = supp.get("asset_type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let filename = supp.get("filename")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let title = supp.get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match asset_type.as_str() {
            "file" | "sourcecode" => {
                let download_url = supp.get("download_urls")
                    .and_then(|d| {
                        if let Some(obj) = d.as_object() {
                            for (_key, val) in obj {
                                if let Some(arr) = val.as_array() {
                                    if let Some(first) = arr.first() {
                                        return first.get("file").and_then(|f| f.as_str()).map(|s| s.to_string());
                                    }
                                }
                            }
                        }
                        None
                    });

                let url = match download_url {
                    Some(u) => u,
                    None => return Ok(0),
                };

                let base = if filename.is_empty() {
                    safe_filename(title)
                } else {
                    safe_filename(filename)
                };
                let safe_name = format!("{:02} - {}", lecture_num, base);
                let file_path = chapter_dir.join(&safe_name);

                if file_exists_with_content(&file_path).await {
                    return Ok(0);
                }

                download_file_simple(&session.client, &url, &file_path).await.ok();
            }
            "externallink" => {
                let external_url = supp.get("external_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !external_url.is_empty() {
                    let base = if filename.is_empty() {
                        safe_filename(title)
                    } else {
                        safe_filename(filename)
                    };
                    let safe_name = format!("{:02} - {}.url", lecture_num, base);
                    let file_path = chapter_dir.join(&safe_name);
                    if !file_exists_with_content(&file_path).await {
                        let content = format!("[InternetShortcut]\nURL={}", external_url);
                        tokio::fs::write(&file_path, content.as_bytes()).await?;
                    }
                }
            }
            _ => {}
        }

        Ok(0)
    }
}

async fn file_exists_with_content(path: &Path) -> bool {
    match tokio::fs::metadata(path).await {
        Ok(m) => m.len() > 0,
        Err(_) => false,
    }
}

async fn download_file_simple(
    client: &reqwest::Client,
    url: &str,
    output_path: &Path,
) -> anyhow::Result<u64> {
    let part_path = output_path.with_extension(
        format!(
            "{}.part",
            output_path.extension().unwrap_or_default().to_string_lossy()
        )
    );

    let result = download_file_inner(client, url, output_path, &part_path).await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&part_path).await;
    }
    result
}

async fn download_file_inner(
    client: &reqwest::Client,
    url: &str,
    output_path: &Path,
    part_path: &Path,
) -> anyhow::Result<u64> {
    let resp = client.get(url)
        .timeout(Duration::from_secs(600))
        .send()
        .await
        .map_err(|e| anyhow!("Download request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(anyhow!("Auth error ({}), session may have expired", status));
        }
        return Err(anyhow!("Download returned status {}", status));
    }

    let mut total: u64 = 0;
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(part_path).await?;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("Stream error: {}", e))?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
        total += chunk.len() as u64;
    }

    tokio::io::AsyncWriteExt::flush(&mut file).await?;
    drop(file);

    tokio::fs::rename(part_path, output_path).await?;

    Ok(total)
}
