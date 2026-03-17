use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::anyhow;
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::core::filename;

use super::api::{self, EstrategiaConcursosCourse, EstrategiaConcursosSession};

#[derive(Debug, Clone, serde::Serialize)]
pub struct EstrategiaConcursosCourseDownloadProgress {
    pub course_id: i64,
    pub course_name: String,
    pub percent: f64,
    pub current_module: String,
    #[serde(rename = "current_page")]
    pub current_lesson: String,
    pub downloaded_bytes: u64,
    #[serde(rename = "total_pages")]
    pub total_lessons: u32,
    #[serde(rename = "completed_pages")]
    pub completed_lessons: u32,
    pub total_modules: u32,
    pub current_module_index: u32,
}

pub async fn download_full_course(
    app: &tauri::AppHandle,
    session: &EstrategiaConcursosSession,
    course: &EstrategiaConcursosCourse,
    output_dir: &str,
    cancel_token: CancellationToken,
) -> anyhow::Result<()> {
    let modules = api::get_course_content(session, course.id).await?;

    if modules.is_empty() {
        return Err(anyhow!(
            "'{}' has no modules available for download",
            course.name
        ));
    }

    let course_dir = format!(
        "{}/{}",
        output_dir,
        filename::sanitize_path_component(&course.name)
    );
    tokio::fs::create_dir_all(&course_dir).await?;

    let total_lessons: usize = modules.iter().map(|m| m.lessons.len()).sum();
    let total_modules = modules.len();
    let total_bytes = Arc::new(AtomicU64::new(0));
    let completed = Arc::new(AtomicUsize::new(0));

    let _ = app.emit(
        "download-progress",
        &EstrategiaConcursosCourseDownloadProgress {
            course_id: course.id,
            course_name: course.name.clone(),
            percent: 0.0,
            current_module: "Starting...".to_string(),
            current_lesson: String::new(),
            downloaded_bytes: 0,
            total_lessons: total_lessons as u32,
            completed_lessons: 0,
            total_modules: total_modules as u32,
            current_module_index: 0,
        },
    );

    for (mi, module) in modules.iter().enumerate() {
        if cancel_token.is_cancelled() {
            return Err(anyhow!("Download cancelled by user"));
        }

        let mod_name = filename::sanitize_path_component(&module.name);
        let mod_dir = format!("{}/{}. {}", course_dir, mi + 1, mod_name);
        tokio::fs::create_dir_all(&mod_dir).await?;

        for (li, lesson) in module.lessons.iter().enumerate() {
            if cancel_token.is_cancelled() {
                return Err(anyhow!("Download cancelled by user"));
            }

            if !lesson.is_disponivel {
                let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                let _ = app.emit(
                    "download-progress",
                    &EstrategiaConcursosCourseDownloadProgress {
                        course_id: course.id,
                        course_name: course.name.clone(),
                        percent: done as f64 / total_lessons as f64 * 100.0,
                        current_module: module.name.clone(),
                        current_lesson: lesson.name.clone(),
                        downloaded_bytes: total_bytes.load(Ordering::Relaxed),
                        total_lessons: total_lessons as u32,
                        completed_lessons: done as u32,
                        total_modules: total_modules as u32,
                        current_module_index: (mi + 1) as u32,
                    },
                );
                continue;
            }

            let lesson_name = filename::sanitize_path_component(&lesson.name);

            let detail = match api::get_lesson_detail(session, lesson.id).await {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!(
                        "[estrategia_concursos] Failed to get lesson detail '{}': {}. Continuing...",
                        lesson.name,
                        e
                    );
                    let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    let _ = app.emit(
                        "download-progress",
                        &EstrategiaConcursosCourseDownloadProgress {
                            course_id: course.id,
                            course_name: course.name.clone(),
                            percent: done as f64 / total_lessons as f64 * 100.0,
                            current_module: module.name.clone(),
                            current_lesson: lesson.name.clone(),
                            downloaded_bytes: total_bytes.load(Ordering::Relaxed),
                            total_lessons: total_lessons as u32,
                            completed_lessons: done as u32,
                            total_modules: total_modules as u32,
                            current_module_index: (mi + 1) as u32,
                        },
                    );
                    continue;
                }
            };

            for (vi, video) in detail.videos.iter().enumerate() {
                if cancel_token.is_cancelled() {
                    return Err(anyhow!("Download cancelled by user"));
                }

                let suffix = if detail.videos.len() > 1 {
                    format!(" ({})", vi + 1)
                } else {
                    String::new()
                };

                let video_path = format!(
                    "{}/{}. {}{}.mp4",
                    mod_dir,
                    li + 1,
                    lesson_name,
                    suffix
                );

                if tokio::fs::try_exists(&video_path).await.unwrap_or(false) {
                    let meta = tokio::fs::metadata(&video_path).await;
                    if meta.map(|m| m.len() > 0).unwrap_or(false) {
                        tracing::info!("[estrategia_concursos] Skipping existing: {}", video_path);
                        continue;
                    }
                }

                match download_file_direct(&session.client, &video.url, &video_path, &cancel_token).await {
                    Ok(size) => {
                        total_bytes.fetch_add(size, Ordering::Relaxed);
                    }
                    Err(e) => {
                        tracing::error!(
                            "[estrategia_concursos] Video download failed for '{}': {}",
                            lesson.name,
                            e
                        );
                    }
                }
            }

            if let Some(ref pdf_url) = detail.pdf_url {
                let pdf_path = format!("{}/{}. {}.pdf", mod_dir, li + 1, lesson_name);
                if !tokio::fs::try_exists(&pdf_path).await.unwrap_or(false) {
                    match download_file_direct(&session.client, pdf_url, &pdf_path, &cancel_token).await {
                        Ok(size) => {
                            total_bytes.fetch_add(size, Ordering::Relaxed);
                        }
                        Err(e) => {
                            tracing::error!(
                                "[estrategia_concursos] PDF download failed for '{}': {}",
                                lesson.name,
                                e
                            );
                        }
                    }
                }
            }

            if let Some(ref pdf_url) = detail.pdf_grifado_url {
                let pdf_path = format!("{}/{}. {} (grifado).pdf", mod_dir, li + 1, lesson_name);
                if !tokio::fs::try_exists(&pdf_path).await.unwrap_or(false) {
                    match download_file_direct(&session.client, pdf_url, &pdf_path, &cancel_token).await {
                        Ok(size) => {
                            total_bytes.fetch_add(size, Ordering::Relaxed);
                        }
                        Err(e) => {
                            tracing::error!(
                                "[estrategia_concursos] PDF grifado download failed for '{}': {}",
                                lesson.name,
                                e
                            );
                        }
                    }
                }
            }

            if let Some(ref pdf_url) = detail.pdf_simplificado_url {
                let pdf_path = format!("{}/{}. {} (simplificado).pdf", mod_dir, li + 1, lesson_name);
                if !tokio::fs::try_exists(&pdf_path).await.unwrap_or(false) {
                    match download_file_direct(&session.client, pdf_url, &pdf_path, &cancel_token).await {
                        Ok(size) => {
                            total_bytes.fetch_add(size, Ordering::Relaxed);
                        }
                        Err(e) => {
                            tracing::error!(
                                "[estrategia_concursos] PDF simplificado download failed for '{}': {}",
                                lesson.name,
                                e
                            );
                        }
                    }
                }
            }

            let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
            let _ = app.emit(
                "download-progress",
                &EstrategiaConcursosCourseDownloadProgress {
                    course_id: course.id,
                    course_name: course.name.clone(),
                    percent: done as f64 / total_lessons as f64 * 100.0,
                    current_module: module.name.clone(),
                    current_lesson: lesson.name.clone(),
                    downloaded_bytes: total_bytes.load(Ordering::Relaxed),
                    total_lessons: total_lessons as u32,
                    completed_lessons: done as u32,
                    total_modules: total_modules as u32,
                    current_module_index: (mi + 1) as u32,
                },
            );
        }
    }

    if cancel_token.is_cancelled() {
        return Err(anyhow!("Download cancelled by user"));
    }

    Ok(())
}

async fn download_file_direct(
    client: &reqwest::Client,
    url: &str,
    output_path: &str,
    cancel_token: &CancellationToken,
) -> anyhow::Result<u64> {
    let out = Path::new(output_path);
    let (tx, _rx) = mpsc::channel(8);

    crate::core::direct_downloader::download_direct(
        client,
        url,
        out,
        tx,
        Some(cancel_token),
    )
    .await
}
