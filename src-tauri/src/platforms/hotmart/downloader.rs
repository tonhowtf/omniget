use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::core::filename;
use crate::core::media_processor::MediaProcessor;
use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo, MediaType};
use crate::models::settings::DownloadSettings;
use crate::platforms::traits::PlatformDownloader;

use super::api::{self, Course, Lesson};
use super::auth::HotmartSession;
use super::parser::{self, DetectedPlayer};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Debug, Clone, serde::Serialize)]
pub struct CourseDownloadProgress {
    pub course_id: u64,
    pub course_name: String,
    pub percent: f64,
    pub current_module: String,
    pub current_page: String,
    pub downloaded_bytes: u64,
    pub total_pages: u32,
    pub completed_pages: u32,
    pub total_modules: u32,
    pub current_module_index: u32,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct DoneManifest {
    file: String,
    size: u64,
    segments: usize,
    completed_at: String,
}

fn done_path(file_path: &str) -> PathBuf {
    let p = PathBuf::from(file_path);
    let file_name = p.file_name().unwrap_or_default().to_string_lossy();
    let manifest_name = format!("{}.omniget.done", file_name);
    p.with_file_name(manifest_name)
}

async fn write_done_manifest(file_path: &str, size: u64, segments: usize) -> anyhow::Result<()> {
    let p = PathBuf::from(file_path);
    let file_name = p
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let manifest = DoneManifest {
        file: file_name,
        size,
        segments,
        completed_at: chrono::Utc::now().to_rfc3339(),
    };
    let json = serde_json::to_string_pretty(&manifest)?;
    tokio::fs::write(done_path(file_path), json).await?;
    Ok(())
}

async fn is_hls_file_valid(file_path: &str) -> bool {
    let dp = done_path(file_path);

    let manifest_bytes = match tokio::fs::read(&dp).await {
        Ok(b) => b,
        Err(_) => return false,
    };

    let manifest: DoneManifest = match serde_json::from_slice(&manifest_bytes) {
        Ok(m) => m,
        Err(_) => return false,
    };

    let meta = match tokio::fs::metadata(file_path).await {
        Ok(m) => m,
        Err(_) => return false,
    };

    meta.len() == manifest.size
}

pub struct HotmartDownloader {
    session: Arc<Mutex<Option<HotmartSession>>>,
    download_settings: DownloadSettings,
    max_concurrent_segments: u32,
    max_retries: u32,
}

impl HotmartDownloader {
    pub fn new(
        session: Arc<Mutex<Option<HotmartSession>>>,
        download_settings: DownloadSettings,
        max_concurrent_segments: u32,
        max_retries: u32,
    ) -> Self {
        Self {
            session,
            download_settings,
            max_concurrent_segments,
            max_retries,
        }
    }

    pub async fn download_lesson(
        &self,
        session: &HotmartSession,
        lesson: &Lesson,
        output_dir: &str,
        referer: &str,
        bytes_tx: tokio::sync::mpsc::UnboundedSender<u64>,
        cancel_token: &CancellationToken,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let mut results = Vec::new();

        tokio::fs::create_dir_all(output_dir).await?;

        if lesson.has_media {
            for (i, media) in lesson.medias.iter().enumerate() {
                if cancel_token.is_cancelled() {
                    anyhow::bail!("Download cancelado pelo usuário");
                }

                let assets = match parser::fetch_player_media_assets(&media.url, session).await {
                    Ok(a) => a,
                    Err(_) => continue,
                };

                if media.media_type.to_uppercase().contains("VIDEO") {
                    let m3u8_url = match assets.first().and_then(|a| a.get("url")).and_then(|v| v.as_str()) {
                        Some(url) => url.to_string(),
                        None => continue,
                    };

                    let out = format!("{}/{}. Aula.mp4", output_dir, i + 1);

                    if is_hls_file_valid(&out).await {
                        continue;
                    }

                    if tokio::fs::try_exists(done_path(&out)).await.unwrap_or(false) {
                        let _ = tokio::fs::remove_file(&out).await;
                        let _ = tokio::fs::remove_file(done_path(&out)).await;
                    }

                    let hls_result = retry_hls_download(
                        &m3u8_url,
                        &out,
                        "https://cf-embed.play.hotmart.com/",
                        Some(bytes_tx.clone()),
                        cancel_token,
                        self.max_concurrent_segments,
                        self.max_retries,
                        3,
                    )
                    .await;

                    match hls_result {
                        Ok(hls_result) => {
                            let _ = write_done_manifest(&out, hls_result.file_size, hls_result.segments).await;
                            results.push(hls_result.path);
                        }
                        Err(e) => {
                            tracing::error!("[download] Falha ao baixar vídeo '{}': {}", out, e);
                            let _ = tokio::fs::remove_file(&out).await;
                            if cancel_token.is_cancelled() {
                                return Err(e);
                            }
                            continue;
                        }
                    }
                } else if media.media_type.to_uppercase().contains("AUDIO") {
                    for asset in &assets {
                        let content_type = asset
                            .get("contentType")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if !content_type.to_lowercase().contains("audio") {
                            continue;
                        }
                        let audio_url = match asset.get("url").and_then(|v| v.as_str()) {
                            Some(url) => url,
                            None => continue,
                        };

                        let safe_name = filename::sanitize_path_component(&media.name);
                        let out = format!(
                            "{}/{}. {}",
                            output_dir,
                            i + 1,
                            if safe_name.is_empty() { "Audio.mp4".to_string() } else { safe_name }
                        );

                        if tokio::fs::try_exists(&out).await.unwrap_or(false) {
                            let meta = tokio::fs::metadata(&out).await;
                            if meta.map(|m| m.len() > 0).unwrap_or(false) {
                                continue;
                            }
                        }

                        let bytes = session.client
                            .get(audio_url)
                            .send()
                            .await?
                            .bytes()
                            .await?;
                        let _ = bytes_tx.send(bytes.len() as u64);
                        tokio::fs::write(&out, &bytes).await?;
                        results.push(PathBuf::from(out));
                    }
                }
            }
        }

        if let Some(html) = &lesson.content {
            let players = parser::detect_players_from_html(html);
            for (i, player) in players.iter().enumerate() {
                if cancel_token.is_cancelled() {
                    anyhow::bail!("Download cancelado pelo usuário");
                }

                let out = format!("{}/{}. Aula.mp4", output_dir, i + 1);

                match player {
                    DetectedPlayer::Vimeo { embed_url } => {
                        if tokio::fs::try_exists(&out).await.unwrap_or(false) {
                            let meta = tokio::fs::metadata(&out).await;
                            if meta.map(|m| m.len() > 0).unwrap_or(false) {
                                continue;
                            }
                        }

                        match crate::core::ytdlp::ensure_ytdlp().await {
                            Ok(ytdlp_path) => {
                                let out_dir = std::path::Path::new(&out).parent()
                                    .unwrap_or(std::path::Path::new("."));
                                let (vtx, _vrx) = mpsc::channel(8);
                                match crate::core::ytdlp::download_video(
                                    &ytdlp_path,
                                    embed_url,
                                    out_dir,
                                    None,
                                    vtx,
                                    None,
                                    None,
                                    None,
                                    Some(referer),
                                    cancel_token.clone(),
                                    None,
                                ).await {
                                    Ok(result) => {
                                        let _ = bytes_tx.send(result.file_size_bytes);
                                        results.push(result.file_path);
                                    }
                                    Err(e) => {
                                        tracing::error!("[download] Falha Vimeo: {}", e);
                                        cleanup_part_files(out_dir).await;
                                        continue;
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("[download] yt-dlp indisponível para Vimeo: {}", e);
                                continue;
                            }
                        }
                    }
                    DetectedPlayer::PandaVideo { m3u8_url, .. } => {
                        if is_hls_file_valid(&out).await {
                            continue;
                        }

                        if tokio::fs::try_exists(done_path(&out)).await.unwrap_or(false) {
                            let _ = tokio::fs::remove_file(&out).await;
                            let _ = tokio::fs::remove_file(done_path(&out)).await;
                        }

                        let panda_referer = m3u8_url
                            .split("com.br")
                            .next()
                            .unwrap_or("")
                            .to_string()
                            + "com.br";
                        match retry_hls_download(m3u8_url, &out, &panda_referer, Some(bytes_tx.clone()), cancel_token, self.max_concurrent_segments, self.max_retries, 3).await {
                            Ok(hls_result) => {
                                let _ = write_done_manifest(&out, hls_result.file_size, hls_result.segments).await;
                                results.push(hls_result.path);
                            }
                            Err(e) => {
                                tracing::error!("[download] Falha PandaVideo: {}", e);
                                let _ = tokio::fs::remove_file(&out).await;
                                if cancel_token.is_cancelled() {
                                    return Err(e);
                                }
                                continue;
                            }
                        }
                    }
                    DetectedPlayer::YouTube { video_id, .. } => {
                        if tokio::fs::try_exists(&out).await.unwrap_or(false) {
                            let meta = tokio::fs::metadata(&out).await;
                            if meta.map(|m| m.len() > 0).unwrap_or(false) {
                                continue;
                            }
                        }

                        let yt_url = format!("https://www.youtube.com/watch?v={}", video_id);
                        match crate::core::ytdlp::ensure_ytdlp().await {
                            Ok(ytdlp_path) => {
                                let out_dir = std::path::Path::new(&out).parent()
                                    .unwrap_or(std::path::Path::new("."));
                                let (ytx, _yrx) = mpsc::channel(8);
                                match crate::core::ytdlp::download_video(
                                    &ytdlp_path,
                                    &yt_url,
                                    out_dir,
                                    None,
                                    ytx,
                                    None,
                                    None,
                                    None,
                                    None,
                                    cancel_token.clone(),
                                    None,
                                ).await {
                                    Ok(result) => {
                                        let _ = bytes_tx.send(result.file_size_bytes);
                                        results.push(result.file_path);
                                    }
                                    Err(e) => {
                                        tracing::error!("[download] Falha YouTube: {}", e);
                                        cleanup_part_files(out_dir).await;
                                        continue;
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("[download] yt-dlp indisponível: {}", e);
                                continue;
                            }
                        }
                    }
                    DetectedPlayer::HotmartNative { .. } => {}
                    DetectedPlayer::Unknown { .. } => {}
                }
            }
        }

        if cancel_token.is_cancelled() {
            anyhow::bail!("Download cancelado pelo usuário");
        }

        if self.download_settings.download_attachments && !lesson.attachments.is_empty() {
            let mat_dir = format!("{}/Materiais", output_dir);
            tokio::fs::create_dir_all(&mat_dir).await?;
            for att in &lesson.attachments {
                let safe_name = filename::sanitize_path_component(&att.file_name);
                let att_path = format!("{}/{}", mat_dir, safe_name);

                if tokio::fs::try_exists(&att_path).await.unwrap_or(false) {
                    continue;
                }

                match download_attachment(session, &att.file_membership_id, &att_path).await {
                    Ok(()) => {
                        results.push(PathBuf::from(att_path));
                    }
                    Err(_) => {}
                }
            }
        }

        if self.download_settings.download_descriptions {
            if let Some(content) = &lesson.content {
                if !content.trim().is_empty() {
                    let desc_path = format!("{}/Descrição.html", output_dir);
                    if !tokio::fs::try_exists(&desc_path).await.unwrap_or(false) {
                        tokio::fs::write(&desc_path, content).await?;
                    }
                }
            }

            if let Some(readings) = &lesson.complementary_readings {
                if !readings.is_empty() {
                    let reading_path = format!("{}/Leitura complementar.html", output_dir);
                    if !tokio::fs::try_exists(&reading_path).await.unwrap_or(false) {
                        let mut html = String::new();
                        for link in readings {
                            let title = link.title.as_deref().unwrap_or("");
                            let url = link.url.as_deref().unwrap_or("#");
                            html.push_str(&format!("<a href=\"{}\">{}</a><br>\n", url, title));
                        }
                        tokio::fs::write(&reading_path, &html).await?;
                    }
                }
            }
        }

        Ok(results)
    }

    pub async fn download_full_course(
        &self,
        course: &Course,
        base_dir: &str,
        progress: mpsc::Sender<CourseDownloadProgress>,
        cancel_token: CancellationToken,
    ) -> anyhow::Result<()> {
        if course.external_platform {
            return Err(anyhow!("Curso hospedado em plataforma externa"));
        }

        let slug = course
            .slug
            .as_deref()
            .or(course.subdomain.as_deref())
            .ok_or_else(|| anyhow!("Curso sem slug/subdomain: {}", course.name))?;

        let session = {
            let guard = self.session.lock().await;
            guard
                .as_ref()
                .ok_or_else(|| anyhow!("Não autenticado"))?
                .clone()
        };

        let modules = api::get_modules(&session, slug, course.id).await?;

        if modules.is_empty() {
            return Err(anyhow!("'{}' não possui módulos disponíveis para baixar", course.name));
        }

        let course_dir = format!(
            "{}/{} - {}",
            base_dir,
            filename::sanitize_path_component(&course.name),
            filename::sanitize_path_component(&course.seller)
        );
        tokio::fs::create_dir_all(&course_dir).await?;

        let total_pages: usize = modules.iter().map(|m| m.pages.len()).sum();
        let total_modules = modules.len();
        let mut done = 0usize;
        let total_bytes = Arc::new(AtomicU64::new(0));

        let _ = progress
            .send(CourseDownloadProgress {
                course_id: course.id,
                course_name: course.name.clone(),
                percent: 0.0,
                current_module: "Iniciando...".to_string(),
                current_page: String::new(),
                downloaded_bytes: 0,
                total_pages: total_pages as u32,
                completed_pages: 0,
                total_modules: total_modules as u32,
                current_module_index: 0,
            })
            .await;

        let referer = format!("https://{}.club.hotmart.com/", slug);

        'outer: for (mi, module) in modules.iter().enumerate() {
            let mod_name = filename::sanitize_path_component(&module.name);
            let mod_dir = format!("{}/{}. {}", course_dir, mi + 1, mod_name);

            for (pi, page) in module.pages.iter().enumerate() {
                if cancel_token.is_cancelled() {
                    break 'outer;
                }

                let page_name = filename::sanitize_path_component(&page.name);
                let page_dir = format!("{}/{}. {}", mod_dir, pi + 1, page_name);
                tokio::fs::create_dir_all(&page_dir).await?;

                let lesson = match api::get_lesson(&session, slug, course.id, &page.hash).await {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::error!("Falha ao carregar lição '{}': {}. Continuando...", page.name, e);
                        done += 1;
                        let _ = progress
                            .send(CourseDownloadProgress {
                                course_id: course.id,
                                course_name: course.name.clone(),
                                percent: done as f64 / total_pages as f64 * 100.0,
                                current_module: module.name.clone(),
                                current_page: page.name.clone(),
                                downloaded_bytes: total_bytes.load(Ordering::Relaxed),
                                total_pages: total_pages as u32,
                                completed_pages: done as u32,
                                total_modules: total_modules as u32,
                                current_module_index: (mi + 1) as u32,
                            })
                            .await;
                        continue;
                    }
                };

                let (lesson_bytes_tx, mut lesson_bytes_rx) =
                    tokio::sync::mpsc::unbounded_channel::<u64>();
                let total_bytes_ref = total_bytes.clone();
                let accumulator = tokio::spawn(async move {
                    while let Some(n) = lesson_bytes_rx.recv().await {
                        total_bytes_ref.fetch_add(n, Ordering::Relaxed);
                    }
                });

                let lesson_result = self
                    .download_lesson(&session, &lesson, &page_dir, &referer, lesson_bytes_tx, &cancel_token)
                    .await;

                let _ = accumulator.await;

                if let Err(e) = lesson_result {
                    if cancel_token.is_cancelled() {
                        break 'outer;
                    }
                    tracing::error!(
                        "Erro ao baixar página '{}': {}. Continuando...",
                        page.name,
                        e
                    );
                }

                done += 1;
                let _ = progress
                    .send(CourseDownloadProgress {
                        course_id: course.id,
                        course_name: course.name.clone(),
                        percent: done as f64 / total_pages as f64 * 100.0,
                        current_module: module.name.clone(),
                        current_page: page.name.clone(),
                        downloaded_bytes: total_bytes.load(Ordering::Relaxed),
                        total_pages: total_pages as u32,
                        completed_pages: done as u32,
                        total_modules: total_modules as u32,
                        current_module_index: (mi + 1) as u32,
                    })
                    .await;
            }
        }

        if cancel_token.is_cancelled() {
            return Err(anyhow!("Download cancelado pelo usuário"));
        }

        Ok(())
    }
}

async fn retry_hls_download(
    m3u8_url: &str,
    output_path: &str,
    referer: &str,
    bytes_tx: Option<tokio::sync::mpsc::UnboundedSender<u64>>,
    cancel_token: &CancellationToken,
    max_concurrent_segments: u32,
    max_retries: u32,
    max_attempts: u32,
) -> anyhow::Result<crate::core::hls_downloader::HlsDownloadResult> {
    let mut last_err = None;
    for attempt in 0..max_attempts {
        if cancel_token.is_cancelled() {
            anyhow::bail!("Download cancelado pelo usuário");
        }
        match MediaProcessor::download_hls(
            m3u8_url,
            output_path,
            referer,
            bytes_tx.clone(),
            cancel_token.clone(),
            max_concurrent_segments,
            max_retries,
        )
        .await
        {
            Ok(result) => {
                let meta = tokio::fs::metadata(output_path).await;
                if meta.map(|m| m.len() > 0).unwrap_or(false) {
                    return Ok(result);
                }
                last_err = Some(anyhow!("Arquivo de saída vazio após download HLS"));
            }
            Err(e) => {
                if cancel_token.is_cancelled() {
                    return Err(e);
                }
                last_err = Some(e);
            }
        }
        if attempt < max_attempts - 1 {
            let _ = tokio::fs::remove_file(output_path).await;
            let backoff = std::time::Duration::from_secs(2u64.pow(attempt));
            tracing::warn!(
                "[hls-retry] Tentativa {}/{} falhou para '{}', retentando em {:?}",
                attempt + 1,
                max_attempts,
                output_path,
                backoff
            );
            tokio::time::sleep(backoff).await;
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("HLS download falhou após {} tentativas", max_attempts)))
}

async fn cleanup_part_files(dir: &std::path::Path) {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return,
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.ends_with(".part") || name.ends_with(".ytdl") {
                let _ = tokio::fs::remove_file(&path).await;
            }
        }
    }
}

async fn download_attachment(
    session: &HotmartSession,
    file_membership_id: &str,
    output_path: &str,
) -> anyhow::Result<()> {
    let info = api::get_attachment_url(session, file_membership_id).await?;

    if !info.is_drm {
        if info.url.is_empty() {
            return Err(anyhow!("URL de download vazia para attachment {}", file_membership_id));
        }
        let bytes = reqwest::get(&info.url).await?.bytes().await?;
        tokio::fs::write(output_path, &bytes).await?;
    } else {
        let lambda_url = info
            .lambda_url
            .as_deref()
            .ok_or_else(|| anyhow!("lambdaUrl não encontrada para DRM"))?;
        let drm_token = info
            .token
            .as_deref()
            .ok_or_else(|| anyhow!("token DRM não encontrado"))?;

        let signed_url = session
            .client
            .get(lambda_url)
            .header("token", drm_token)
            .header("User-Agent", USER_AGENT)
            .send()
            .await?
            .text()
            .await?;

        if signed_url.is_empty() || signed_url.contains("500") {
            return Err(anyhow!("Anexo DRM indisponível (resposta vazia ou erro 500)"));
        }

        let bytes = reqwest::get(&signed_url).await?.bytes().await?;
        let drm_output = format!(
            "{}/drm_{}",
            std::path::Path::new(output_path)
                .parent()
                .map(|p| p.to_str().unwrap_or("."))
                .unwrap_or("."),
            std::path::Path::new(output_path)
                .file_name()
                .map(|f| f.to_str().unwrap_or("file"))
                .unwrap_or("file")
        );
        tokio::fs::write(&drm_output, &bytes).await?;
    }

    Ok(())
}

#[async_trait]
impl PlatformDownloader for HotmartDownloader {
    fn name(&self) -> &str {
        "hotmart"
    }

    fn can_handle(&self, url: &str) -> bool {
        url.contains("hotmart.com") || url.contains("club.hotmart.com")
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        Ok(MediaInfo {
            title: format!("Hotmart course: {}", url),
            author: String::new(),
            platform: "hotmart".to_string(),
            duration_seconds: None,
            thumbnail_url: None,
            available_qualities: vec![],
            media_type: MediaType::Course,
            file_size_bytes: None,
        })
    }

    async fn download(
        &self,
        _info: &MediaInfo,
        _opts: &DownloadOptions,
        _progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<DownloadResult> {
        Err(anyhow!("Hotmart downloads use start_course_download, not the generic download trait"))
    }
}
