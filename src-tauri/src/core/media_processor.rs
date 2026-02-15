pub struct MediaProcessor;

impl MediaProcessor {
    pub async fn remux(input: &str, output: &str) -> anyhow::Result<()> {
        let status = tokio::process::Command::new("ffmpeg")
            .args(["-y", "-i", input, "-c", "copy", output])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .status()
            .await?;

        if !status.success() {
            anyhow::bail!("FFmpeg remux falhou com status {}", status);
        }

        tracing::info!("Remux completo: {} -> {}", input, output);
        Ok(())
    }

    pub async fn merge_audio_video(
        video: &str,
        audio: &str,
        output: &str,
    ) -> anyhow::Result<()> {
        let status = tokio::process::Command::new("ffmpeg")
            .args([
                "-y", "-i", video, "-i", audio, "-map", "0:v", "-map", "1:a", "-c", "copy",
                output,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .status()
            .await?;

        if !status.success() {
            anyhow::bail!("FFmpeg merge falhou com status {}", status);
        }

        tracing::info!("Merge completo: {} + {} -> {}", video, audio, output);
        Ok(())
    }

    pub async fn download_direct(
        url: &str,
        output: &str,
        headers: &[(&str, &str)],
    ) -> anyhow::Result<()> {
        let mut args = vec!["-y".to_string()];

        if !headers.is_empty() {
            let header_str: String = headers
                .iter()
                .map(|(k, v)| format!("{}: {}\r\n", k, v))
                .collect();
            args.extend(["-headers".to_string(), header_str]);
        }

        args.extend([
            "-i".to_string(),
            url.to_string(),
            "-c".to_string(),
            "copy".to_string(),
            output.to_string(),
        ]);

        let status = tokio::process::Command::new("ffmpeg")
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .status()
            .await?;

        if !status.success() {
            anyhow::bail!("FFmpeg download_direct falhou com status {}", status);
        }

        tracing::info!("Download direto completo: {}", output);
        Ok(())
    }
}
