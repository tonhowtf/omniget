use std::path::Path;

use anyhow::anyhow;

pub async fn is_ffmpeg_available() -> bool {
    tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

pub async fn mux_video_audio(video: &Path, audio: &Path, output: &Path) -> anyhow::Result<()> {
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let status = tokio::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            &video.to_string_lossy(),
            "-i",
            &audio.to_string_lossy(),
            "-c",
            "copy",
            &output.to_string_lossy(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|e| anyhow!("Falha ao executar ffmpeg: {}", e))?;

    if !status.success() {
        return Err(anyhow!("ffmpeg retornou código {}", status));
    }

    Ok(())
}
