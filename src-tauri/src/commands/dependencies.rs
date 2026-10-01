use std::path::PathBuf;

use serde::Serialize;

use crate::core::dependencies;

#[derive(Debug, Clone, Serialize)]
pub struct DependencyStatus {
    pub name: String,
    pub installed: bool,
    pub version: Option<String>,
    pub source: String,
    pub path: Option<String>,
    pub outdated: bool,
}

#[tauri::command]
pub async fn check_dependencies() -> Result<Vec<DependencyStatus>, String> {
    let (ytdlp_result, ffmpeg_result) = tokio::join!(
        dependencies::find_tool_with_source("yt-dlp"),
        dependencies::find_tool_with_source("ffmpeg"),
    );

    let (ytdlp_version, ffmpeg_version) = tokio::join!(
        async {
            match ytdlp_result.as_ref() {
                Some((path, _)) => dependencies::check_version_at_path(path, "yt-dlp").await,
                None => None,
            }
        },
        async {
            match ffmpeg_result.as_ref() {
                Some((path, _)) => dependencies::check_version_at_path(path, "ffmpeg").await,
                None => None,
            }
        },
    );

    let ytdlp_source = ytdlp_result
        .as_ref()
        .map(|(_, s)| s.to_string())
        .unwrap_or_else(|| "missing".to_string());
    let ytdlp_path = ytdlp_result.map(|(p, _)| p.to_string_lossy().to_string());

    let ffmpeg_source = ffmpeg_result
        .as_ref()
        .map(|(_, s)| s.to_string())
        .unwrap_or_else(|| "missing".to_string());
    let ffmpeg_path = ffmpeg_result.map(|(p, _)| p.to_string_lossy().to_string());

    let ytdlp_outdated = ytdlp_version
        .as_deref()
        .and_then(crate::core::ytdlp::ytdlp_version_is_supported)
        .map(|supported| !supported)
        .unwrap_or(false);

    Ok(vec![
        DependencyStatus {
            name: "yt-dlp".into(),
            installed: ytdlp_version.is_some(),
            version: ytdlp_version,
            source: ytdlp_source,
            path: ytdlp_path,
            outdated: ytdlp_outdated,
        },
        DependencyStatus {
            name: "FFmpeg".into(),
            installed: ffmpeg_version.is_some(),
            version: ffmpeg_version,
            source: ffmpeg_source,
            path: ffmpeg_path,
            outdated: false,
        },
    ])
}

#[tauri::command]
pub async fn check_ytdlp_available() -> Result<bool, String> {
    Ok(crate::core::ytdlp::find_ytdlp_cached().await.is_some())
}

#[tauri::command]
pub async fn install_dependency(name: String, force: Option<bool>) -> Result<String, String> {
    let force = force.unwrap_or(false);
    match name.as_str() {
        "yt-dlp" => {
            if force {
                archive_before_update("yt-dlp").await;
                crate::core::ytdlp::update_ytdlp()
                    .await
                    .map_err(|e| e.to_string())?;
            } else {
                crate::core::ytdlp::ensure_ytdlp()
                    .await
                    .map_err(|e| e.to_string())?;
            }
            crate::core::ytdlp::reset_ytdlp_cache();
        }
        "FFmpeg" => {
            if force {
                dependencies::update_ffmpeg()
                    .await
                    .map_err(|e| e.to_string())?;
            } else {
                dependencies::ensure_ffmpeg()
                    .await
                    .map_err(|e| e.to_string())?;
            }
            crate::core::ytdlp::reset_ffmpeg_location_cache();
            crate::core::ffmpeg::reset_ffmpeg_available_cache();
        }
        _ => return Err(format!("Unknown dependency: {}", name)),
    }

    dependencies::check_version(match name.as_str() {
        "FFmpeg" => "ffmpeg",
        other => other,
    })
    .await
    .ok_or_else(|| "Installed but version check failed".into())
}

async fn managed_binary_path(name: &str) -> Option<PathBuf> {
    match name {
        "yt-dlp" => omniget_core::core::ytdlp::managed_ytdlp_path(),
        _ => None,
    }
}

fn agora_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

async fn archive_before_update(name: &str) {
    let Some(bin) = managed_binary_path(name).await else {
        return;
    };
    match crate::core::binary_versions::archive_current(&bin, agora_ms()) {
        Ok(Some(dest)) => {
            tracing::info!("[deps] {} arquivado em {}", name, dest.display());
            let arquivadas = crate::core::binary_versions::list_archived(&bin);
            for velha in crate::core::binary_versions::prune_list(
                &arquivadas,
                crate::core::binary_versions::KEEP_VERSIONS,
            ) {
                let _ = std::fs::remove_file(&velha);
            }
        }
        Ok(None) => {}
        Err(e) => tracing::warn!("[deps] nao foi possivel arquivar {}: {}", name, e),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchivedVersion {
    pub stamp_ms: String,
    pub path: String,
}

#[tauri::command]
pub async fn dependency_archived_versions(name: String) -> Result<Vec<ArchivedVersion>, String> {
    let Some(bin) = managed_binary_path(&name).await else {
        return Ok(Vec::new());
    };
    let mut lista: Vec<ArchivedVersion> = crate::core::binary_versions::list_archived(&bin)
        .into_iter()
        .map(|(stamp, path)| ArchivedVersion {
            stamp_ms: stamp.to_string(),
            path: path.to_string_lossy().to_string(),
        })
        .collect();
    lista.reverse();
    Ok(lista)
}

#[tauri::command]
pub async fn rollback_dependency(name: String, stamp_ms: String) -> Result<String, String> {
    let alvo: u128 = stamp_ms
        .parse()
        .map_err(|_| format!("stamp invalido: {stamp_ms}"))?;
    let bin = managed_binary_path(&name)
        .await
        .ok_or_else(|| format!("{name} nao tem binario gerenciado"))?;

    let arquivada = crate::core::binary_versions::list_archived(&bin)
        .into_iter()
        .find(|(s, _)| *s == alvo)
        .map(|(_, p)| p)
        .ok_or_else(|| "versao guardada nao encontrada".to_string())?;

    crate::core::binary_versions::rollback_to(&bin, &arquivada).map_err(|e| e.to_string())?;

    if name == "yt-dlp" {
        crate::core::ytdlp::reset_ytdlp_cache();
    }

    crate::core::dependencies::check_version(&name)
        .await
        .ok_or_else(|| "voltou, mas a checagem de versao falhou".to_string())
}

#[tauri::command]
pub fn dependency_install_dir(name: String) -> Result<String, String> {
    let dir = match name.as_str() {
        "yt-dlp" | "FFmpeg" => crate::core::paths::app_data_dir()
            .ok_or_else(|| "could not determine app data dir".to_string())?
            .join("bin"),
        _ => return Err(format!("Unknown dependency: {}", name)),
    };
    Ok(dir.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn set_dependency_path(name: String, source_path: String) -> Result<String, String> {
    let src = PathBuf::from(&source_path);

    omniget_core::core::binary_overrides::set(&name, &src)?;

    if name == "yt-dlp" {
        crate::core::ytdlp::reset_ytdlp_cache();
    }
    if name == "FFmpeg" {
        crate::core::ytdlp::reset_ffmpeg_location_cache();
        crate::core::ffmpeg::reset_ffmpeg_available_cache();
    }

    Ok(
        crate::core::dependencies::check_version(match name.as_str() {
            "FFmpeg" => "ffmpeg",
            other => other,
        })
        .await
        .unwrap_or_else(|| "custom".to_string()),
    )
}

#[tauri::command]
pub async fn clear_dependency_path(name: String) -> Result<(), String> {
    omniget_core::core::binary_overrides::clear(&name)?;
    if name == "yt-dlp" {
        crate::core::ytdlp::reset_ytdlp_cache();
    }
    if name == "FFmpeg" {
        crate::core::ytdlp::reset_ffmpeg_location_cache();
        crate::core::ffmpeg::reset_ffmpeg_available_cache();
    }
    Ok(())
}

#[tauri::command]
pub fn dependency_custom_path(name: String) -> Option<String> {
    omniget_core::core::binary_overrides::get(&name).map(|p| p.to_string_lossy().to_string())
}
