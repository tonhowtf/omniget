use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex};

use crate::core::filename;
use crate::core::media_processor::MediaProcessor;
use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo, MediaType};
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
}

pub struct HotmartDownloader {
    session: Arc<Mutex<Option<HotmartSession>>>,
}

impl HotmartDownloader {
    pub fn new(session: Arc<Mutex<Option<HotmartSession>>>) -> Self {
        Self { session }
    }

    pub async fn download_lesson(
        &self,
        lesson: &Lesson,
        output_dir: &str,
        referer: &str,
        _progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let session_guard = self.session.lock().await;
        let session = session_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Não autenticado"))?;
        let mut results = Vec::new();

        tokio::fs::create_dir_all(output_dir).await?;

        if lesson.has_player_media {
            for (i, media) in lesson.medias_src.iter().enumerate() {
                let assets = match parser::fetch_player_media_assets(&media.media_src_url, session).await {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::warn!("Falha ao extrair mediaAssets de '{}': {}", media.media_name, e);
                        continue;
                    }
                };

                if media.media_type.to_uppercase().contains("VIDEO") {
                    let m3u8_url = match assets.first().and_then(|a| a.get("url")).and_then(|v| v.as_str()) {
                        Some(url) => url.to_string(),
                        None => {
                            tracing::warn!("m3u8 URL não encontrada para '{}'", media.media_name);
                            continue;
                        }
                    };

                    tracing::info!("[download] m3u8 URL extraída: {}", m3u8_url);

                    let safe_name = filename::sanitize_path_component(&media.media_name);
                    let out = format!(
                        "{}/{}. {}.mp4",
                        output_dir,
                        i + 1,
                        if safe_name.is_empty() { "Aula".to_string() } else { safe_name }
                    );

                    if tokio::fs::try_exists(&out).await.unwrap_or(false) {
                        tracing::info!("[download] Já existe, pulando: {}", out);
                        continue;
                    }

                    tracing::info!("[download] Baixando vídeo: {}", out);
                    if let Err(e) = MediaProcessor::download_hls_ffmpeg(
                        &m3u8_url,
                        &out,
                        "https://cf-embed.play.hotmart.com/",
                    )
                    .await
                    {
                        tracing::error!("[download] Falha ao baixar vídeo '{}': {}", out, e);
                        continue;
                    }
                    results.push(PathBuf::from(out));
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

                        let safe_name = filename::sanitize_path_component(&media.media_name);
                        let out = format!(
                            "{}/{}. {}",
                            output_dir,
                            i + 1,
                            if safe_name.is_empty() { "Audio.mp4".to_string() } else { safe_name }
                        );

                        if tokio::fs::try_exists(&out).await.unwrap_or(false) {
                            tracing::info!("[download] Já existe, pulando: {}", out);
                            continue;
                        }

                        tracing::info!("[download] Baixando áudio: {}", out);
                        let bytes = reqwest::Client::new()
                            .get(audio_url)
                            .header("User-Agent", USER_AGENT)
                            .send()
                            .await?
                            .bytes()
                            .await?;
                        tokio::fs::write(&out, &bytes).await?;
                        results.push(PathBuf::from(out));
                    }
                }
            }
        }

        if let Some(html) = &lesson.content {
            let players = parser::detect_players_from_html(html);
            for (i, player) in players.iter().enumerate() {
                let out = format!("{}/{}. Aula.mp4", output_dir, i + 1);

                if tokio::fs::try_exists(&out).await.unwrap_or(false) {
                    tracing::info!("[download] Já existe, pulando: {}", out);
                    continue;
                }

                match player {
                    DetectedPlayer::Vimeo { embed_url } => {
                        tracing::info!("[download] Baixando Vimeo: {}", embed_url);
                        if let Err(e) = MediaProcessor::download_hls_ffmpeg(embed_url, &out, referer).await {
                            tracing::error!("[download] Falha Vimeo: {}", e);
                            continue;
                        }
                        results.push(PathBuf::from(&out));
                    }
                    DetectedPlayer::PandaVideo { m3u8_url, .. } => {
                        let panda_referer = m3u8_url
                            .split("com.br")
                            .next()
                            .unwrap_or("")
                            .to_string()
                            + "com.br";
                        tracing::info!("[download] Baixando PandaVideo: {}", m3u8_url);
                        if let Err(e) = MediaProcessor::download_hls_ffmpeg(m3u8_url, &out, &panda_referer).await {
                            tracing::error!("[download] Falha PandaVideo: {}", e);
                            continue;
                        }
                        results.push(PathBuf::from(&out));
                    }
                    DetectedPlayer::YouTube { video_id, .. } => {
                        tracing::info!("[download] Baixando YouTube: {}", video_id);
                        let video = rusty_ytdl::Video::new(video_id)
                            .map_err(|e| anyhow!("rusty_ytdl: {}", e))?;
                        if let Err(e) = video.download(&out).await {
                            tracing::error!("[download] Falha YouTube: {}", e);
                            continue;
                        }
                        results.push(PathBuf::from(&out));
                    }
                    DetectedPlayer::HotmartNative { .. } => {}
                    DetectedPlayer::Unknown { src } => {
                        tracing::warn!("[download] Player desconhecido ignorado: {}", src);
                    }
                }
            }
        }

        if !lesson.attachments.is_empty() {
            let mat_dir = format!("{}/Materiais", output_dir);
            tokio::fs::create_dir_all(&mat_dir).await?;
            for att in &lesson.attachments {
                let safe_name = filename::sanitize_path_component(&att.file_name);
                let att_path = format!("{}/{}", mat_dir, safe_name);

                if tokio::fs::try_exists(&att_path).await.unwrap_or(false) {
                    tracing::info!("[download] Anexo já existe, pulando: {}", att_path);
                    continue;
                }

                match download_attachment(session, &att.file_membership_id, &att_path).await {
                    Ok(()) => {
                        tracing::info!("[download] Anexo salvo: {}", att_path);
                        results.push(PathBuf::from(att_path));
                    }
                    Err(e) => {
                        tracing::warn!("[download] Falha ao baixar anexo '{}': {}", att.file_name, e);
                    }
                }
            }
        }

        if let Some(content) = &lesson.content {
            if !content.trim().is_empty() {
                let desc_path = format!("{}/Descrição.html", output_dir);
                if !tokio::fs::try_exists(&desc_path).await.unwrap_or(false) {
                    tokio::fs::write(&desc_path, content).await?;
                    tracing::info!("[download] Descrição salva: {}", desc_path);
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
                    tracing::info!("[download] Leitura complementar salva: {}", reading_path);
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
    ) -> anyhow::Result<()> {
        let session_guard = self.session.lock().await;
        let session = session_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Não autenticado"))?;

        if course.external_platform {
            return Err(anyhow!("Curso hospedado em plataforma externa"));
        }

        let slug = course
            .slug
            .as_deref()
            .or(course.subdomain.as_deref())
            .ok_or_else(|| anyhow!("Curso sem slug/subdomain: {}", course.name))?;

        let modules = api::get_modules(session, slug, course.id).await?;

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
        let mut done = 0usize;

        tracing::info!(
            "{} módulos encontrados para '{}'",
            modules.len(),
            course.name,
        );

        drop(session_guard);

        let referer = format!("https://{}.club.hotmart.com/", slug);

        for (mi, module) in modules.iter().enumerate() {
            let mod_name = filename::sanitize_path_component(&module.name);
            let mod_dir = format!("{}/{}. {}", course_dir, mi + 1, mod_name);

            for (pi, page) in module.pages.iter().enumerate() {
                let page_name = filename::sanitize_path_component(&page.name);
                let page_dir = format!("{}/{}. {}", mod_dir, pi + 1, page_name);
                tokio::fs::create_dir_all(&page_dir).await?;

                tracing::info!(
                    "[{}/{}] Módulo '{}', Página: '{}'",
                    done + 1,
                    total_pages,
                    module.name,
                    page.name
                );

                let session_guard = self.session.lock().await;
                let session = session_guard
                    .as_ref()
                    .ok_or_else(|| anyhow!("Sessão perdida durante download"))?;

                let lesson = match api::get_lesson(session, slug, course.id, &page.hash).await {
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
                            })
                            .await;
                        continue;
                    }
                };
                drop(session_guard);

                let (lesson_tx, _lesson_rx) = mpsc::channel(10);

                if let Err(e) = self
                    .download_lesson(&lesson, &page_dir, &referer, lesson_tx)
                    .await
                {
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
                    })
                    .await;
            }
        }

        tracing::info!("Download completo do curso '{}'", course.name);
        Ok(())
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
        })
    }

    async fn download(
        &self,
        _info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<DownloadResult> {
        let output = opts.output_dir.join("hotmart_download.mp4");
        let _ = progress.send(100.0).await;
        Ok(DownloadResult {
            file_path: output,
            file_size_bytes: 0,
            duration_seconds: 0.0,
        })
    }
}
