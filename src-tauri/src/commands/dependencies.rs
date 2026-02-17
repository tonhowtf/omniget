use serde::Serialize;

use crate::core::dependencies;

#[derive(Debug, Clone, Serialize)]
pub struct DependencyStatus {
    pub name: String,
    pub installed: bool,
    pub version: Option<String>,
}

#[tauri::command]
pub async fn check_dependencies() -> Result<Vec<DependencyStatus>, String> {
    let (ytdlp_version, ffmpeg_version) = tokio::join!(
        dependencies::check_version("yt-dlp"),
        dependencies::check_version("ffmpeg"),
    );

    Ok(vec![
        DependencyStatus {
            name: "yt-dlp".into(),
            installed: ytdlp_version.is_some(),
            version: ytdlp_version,
        },
        DependencyStatus {
            name: "FFmpeg".into(),
            installed: ffmpeg_version.is_some(),
            version: ffmpeg_version,
        },
    ])
}

#[tauri::command]
pub async fn install_dependency(name: String) -> Result<String, String> {
    match name.as_str() {
        "yt-dlp" => {
            crate::core::ytdlp::ensure_ytdlp()
                .await
                .map_err(|e| e.to_string())?;
        }
        "FFmpeg" => {
            dependencies::ensure_ffmpeg()
                .await
                .map_err(|e| e.to_string())?;
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
