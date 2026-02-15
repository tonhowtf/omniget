use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex};

use crate::core::hls_downloader::HlsDownloader;
use crate::core::media_processor::MediaProcessor;
use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo, MediaType};
use crate::platforms::traits::PlatformDownloader;

use super::api::{self, Course, Lesson};
use super::auth::HotmartSession;
use super::parser::{self, DetectedPlayer};

#[derive(Debug, Clone, serde::Serialize)]
pub struct CourseDownloadProgress {
    pub course_id: u64,
    pub course_name: String,
    pub percent: f64,
    pub current_module: String,
    pub current_page: String,
}

pub struct HotmartDownloader {
    hls: Arc<HlsDownloader>,
    session: Arc<Mutex<Option<HotmartSession>>>,
}

impl HotmartDownloader {
    pub fn new(session: Arc<Mutex<Option<HotmartSession>>>) -> Self {
        Self {
            hls: Arc::new(HlsDownloader::new()),
            session,
        }
    }

    pub async fn download_lesson(
        &self,
        lesson: &Lesson,
        output_dir: &str,
        referer: &str,
        progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let session_guard = self.session.lock().await;
        let session = session_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Não autenticado"))?;
        let mut results = Vec::new();

        if lesson.has_player_media {
            for (i, media) in lesson.medias_src.iter().enumerate() {
                if media
                    .media_type
                    .to_uppercase()
                    .contains("VIDEO")
                {
                    let players =
                        parser::extract_native_urls(&[media.clone()], session).await;
                    for player in players {
                        if let DetectedPlayer::HotmartNative { m3u8_url, name } = player {
                            let safe_name = sanitize_filename::sanitize(&name);
                            let out = format!(
                                "{}/{}. {}.mp4",
                                output_dir,
                                i + 1,
                                if safe_name.is_empty() {
                                    "Aula".to_string()
                                } else {
                                    safe_name
                                }
                            );
                            tracing::info!("Baixando HLS nativo: {} -> {}", &m3u8_url[..60.min(m3u8_url.len())], out);
                            self.hls
                                .download_hls(
                                    &m3u8_url,
                                    &out,
                                    &[("Referer", "https://cf-embed.play.hotmart.com/")],
                                    "720",
                                    Some(progress.clone()),
                                )
                                .await?;
                            results.push(PathBuf::from(out));
                        }
                    }
                }

                if media
                    .media_type
                    .to_uppercase()
                    .contains("AUDIO")
                {
                    let safe_name = sanitize_filename::sanitize(&media.media_name);
                    let ext = if media.media_src_url.contains(".mp3") {
                        "mp3"
                    } else {
                        "m4a"
                    };
                    let out = format!(
                        "{}/{}. {}.{}",
                        output_dir,
                        i + 1,
                        if safe_name.is_empty() {
                            "Audio".to_string()
                        } else {
                            safe_name
                        },
                        ext,
                    );
                    tracing::info!("Baixando áudio direto: {}", media.media_src_url);
                    let bytes = session
                        .client
                        .get(&media.media_src_url)
                        .send()
                        .await?
                        .bytes()
                        .await?;
                    tokio::fs::create_dir_all(output_dir).await?;
                    tokio::fs::write(&out, &bytes).await?;
                    results.push(PathBuf::from(out));
                }
            }
        }

        if let Some(html) = &lesson.content {
            let players = parser::detect_players_from_html(html);
            for (i, player) in players.iter().enumerate() {
                let out = format!("{}/{}. Aula.mp4", output_dir, i + 1);
                match player {
                    DetectedPlayer::Vimeo { embed_url } => {
                        tracing::info!("Baixando Vimeo via FFmpeg: {}", embed_url);
                        MediaProcessor::download_direct(
                            embed_url,
                            &out,
                            &[("Referer", referer)],
                        )
                        .await?;
                        results.push(PathBuf::from(&out));
                    }
                    DetectedPlayer::PandaVideo { m3u8_url, .. } => {
                        let panda_referer = m3u8_url
                            .split("com.br")
                            .next()
                            .unwrap_or("")
                            .to_string()
                            + "com.br";
                        tracing::info!("Baixando PandaVideo HLS: {}", &m3u8_url[..60.min(m3u8_url.len())]);
                        self.hls
                            .download_hls(
                                m3u8_url,
                                &out,
                                &[
                                    ("Referer", &panda_referer),
                                    ("User-Agent", "Mozilla/5.0"),
                                ],
                                "720",
                                Some(progress.clone()),
                            )
                            .await?;
                        results.push(PathBuf::from(&out));
                    }
                    DetectedPlayer::YouTube { video_id, .. } => {
                        tracing::info!("Baixando YouTube via rusty_ytdl: {}", video_id);
                        let video = rusty_ytdl::Video::new(video_id)
                            .map_err(|e| anyhow!("rusty_ytdl: {}", e))?;
                        video
                            .download(&out)
                            .await
                            .map_err(|e| anyhow!("YouTube download: {}", e))?;
                        results.push(PathBuf::from(&out));
                    }
                    DetectedPlayer::HotmartNative { .. } => {}
                    DetectedPlayer::Unknown { src } => {
                        tracing::warn!("Player desconhecido ignorado: {}", src);
                    }
                }
            }
        }

        if !lesson.attachments.is_empty() {
            let mat_dir = format!("{}/Materiais", output_dir);
            tokio::fs::create_dir_all(&mat_dir).await?;
            for att in &lesson.attachments {
                match api::get_attachment_url(session, &att.file_membership_id).await {
                    Ok(info) => {
                        let bytes =
                            session.client.get(&info.url).send().await?.bytes().await?;
                        let att_path = format!(
                            "{}/{}",
                            mat_dir,
                            sanitize_filename::sanitize(&att.file_name)
                        );
                        tokio::fs::write(&att_path, &bytes).await?;
                        tracing::info!("Anexo salvo: {}", att_path);
                        results.push(PathBuf::from(att_path));
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Falha ao baixar anexo {}: {}",
                            att.file_name,
                            e
                        );
                    }
                }
            }
        }

        if let Some(content) = &lesson.content {
            if !content.trim().is_empty() {
                let desc_path = format!("{}/Descricao.html", output_dir);
                tokio::fs::write(&desc_path, content).await?;
                tracing::info!("Descrição salva: {}", desc_path);
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

        let slug = course
            .slug
            .as_deref()
            .or(course.subdomain.as_deref())
            .ok_or_else(|| anyhow!("Curso sem slug/subdomain: {}", course.name))?;

        let modules = api::get_modules(session, slug, course.id).await?;

        let course_dir = format!(
            "{}/{} - {}",
            base_dir,
            sanitize_filename::sanitize(&course.name),
            sanitize_filename::sanitize(&course.seller)
        );
        tokio::fs::create_dir_all(&course_dir).await?;

        let total_pages: usize = modules.iter().map(|m| m.pages.len()).sum();
        let mut done = 0usize;

        tracing::info!(
            "Iniciando download do curso '{}': {} módulos, {} páginas",
            course.name,
            modules.len(),
            total_pages,
        );

        drop(session_guard);

        for (mi, module) in modules.iter().enumerate() {
            let mod_dir = format!(
                "{}/{}. {}",
                course_dir,
                mi + 1,
                sanitize_filename::sanitize(&module.name)
            );

            for (pi, page) in module.pages.iter().enumerate() {
                let page_dir = format!(
                    "{}/{}. {}",
                    mod_dir,
                    pi + 1,
                    sanitize_filename::sanitize(&page.name)
                );
                tokio::fs::create_dir_all(&page_dir).await?;

                tracing::info!(
                    "[{}/{}] Módulo {}, Página: {}",
                    done + 1,
                    total_pages,
                    module.name,
                    page.name
                );

                let session_guard = self.session.lock().await;
                let session = session_guard
                    .as_ref()
                    .ok_or_else(|| anyhow!("Sessão perdida durante download"))?;

                let lesson =
                    api::get_lesson(session, slug, course.id, &page.hash).await?;
                drop(session_guard);

                let referer = format!("https://{}.club.hotmart.com/", slug);
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
