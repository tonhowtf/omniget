use std::path::PathBuf;
use std::process::Stdio;

use anyhow::anyhow;

pub fn is_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

fn managed_bin_dir() -> Option<PathBuf> {
    Some(crate::core::paths::app_data_dir()?.join("bin"))
}

fn bin_name(tool: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{}.exe", tool)
    } else {
        tool.to_string()
    }
}

pub async fn find_tool(tool: &str) -> Option<PathBuf> {
    let _timer_start = std::time::Instant::now();
    let name = bin_name(tool);
    let version_flag = version_flag_for(tool);

    #[cfg(target_os = "linux")]
    {
        let flatpak_path = PathBuf::from("/app/bin").join(&name);
        if flatpak_path.exists() {
            tracing::debug!("[perf] find_tool({}) took {:?}", tool, _timer_start.elapsed());
            return Some(flatpak_path);
        }
    }

    if let Ok(status) = crate::core::process::command(&name)
        .arg(version_flag)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
        if status.success() {
            tracing::debug!("[perf] find_tool({}) took {:?}", tool, _timer_start.elapsed());
            return Some(PathBuf::from(&name));
        }
    }

    let managed = managed_bin_dir()?.join(&name);
    if managed.exists() {
        tracing::debug!("[perf] find_tool({}) took {:?}", tool, _timer_start.elapsed());
        return Some(managed);
    }

    tracing::debug!("[perf] find_tool({}) took {:?}", tool, _timer_start.elapsed());
    None
}

fn version_flag_for(tool: &str) -> &'static str {
    match tool {
        "ffmpeg" | "ffprobe" => "-version",
        _ => "--version",
    }
}

pub async fn check_version(tool: &str) -> Option<String> {
    let _timer_start = std::time::Instant::now();
    let path = find_tool(tool).await?;
    let version_flag = version_flag_for(tool);
    let output = crate::core::process::command(&path)
        .arg(version_flag)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        tracing::debug!("[perf] check_version({}) took {:?}", tool, _timer_start.elapsed());
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next().unwrap_or("");

    let result = if tool == "ffmpeg" || tool == "ffprobe" {
        first_line
            .split_whitespace()
            .nth(2)
            .map(|s| s.to_string())
    } else if tool == "yt-dlp" {
        Some(first_line.trim().to_string())
    } else if tool == "aria2c" {
        first_line
            .split_whitespace()
            .nth(2)
            .map(|s| s.to_string())
    } else {
        Some(first_line.trim().to_string())
    };

    tracing::debug!("[perf] check_version({}) took {:?}", tool, _timer_start.elapsed());
    result
}

pub async fn ensure_ffmpeg() -> anyhow::Result<PathBuf> {
    if let Some(path) = find_tool("ffmpeg").await {
        return Ok(path);
    }
    if is_flatpak() {
        return Err(anyhow!("FFmpeg not found in Flatpak sandbox"));
    }
    let path = download_ffmpeg().await?;
    crate::core::ytdlp::reset_ffmpeg_location_cache();
    Ok(path)
}

async fn download_ffmpeg() -> anyhow::Result<PathBuf> {
    let bin_dir = managed_bin_dir()
        .ok_or_else(|| anyhow!("Could not determine data directory"))?;
    tokio::fs::create_dir_all(&bin_dir).await?;

    let ffmpeg_name = bin_name("ffmpeg");
    let ffprobe_name = bin_name("ffprobe");
    let ffmpeg_target = bin_dir.join(&ffmpeg_name);

    let (url, archive_type) = ffmpeg_download_url();

    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!("Failed to download FFmpeg: HTTP {}", response.status()));
    }

    let bytes = response.bytes().await?;

    match archive_type {
        ArchiveType::Zip => extract_zip_ffmpeg(&bytes, &bin_dir, &ffmpeg_name, &ffprobe_name).await?,
        ArchiveType::TarXz => extract_tar_xz_ffmpeg(&bytes, &bin_dir, &ffmpeg_name, &ffprobe_name).await?,
        ArchiveType::SingleBinary => {
            tokio::fs::write(&ffmpeg_target, &bytes).await?;
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        let _ = tokio::fs::set_permissions(&ffmpeg_target, perms.clone()).await;
        let ffprobe_path = bin_dir.join(&ffprobe_name);
        if ffprobe_path.exists() {
            let _ = tokio::fs::set_permissions(&ffprobe_path, perms).await;
        }
    }

    if !ffmpeg_target.exists() {
        return Err(anyhow!("FFmpeg binary not found after extraction"));
    }

    Ok(ffmpeg_target)
}

enum ArchiveType {
    Zip,
    TarXz,
    SingleBinary,
}

fn ffmpeg_download_url() -> (&'static str, ArchiveType) {
    if cfg!(target_os = "windows") {
        (
            "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip",
            ArchiveType::Zip,
        )
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            (
                "https://www.osxexperts.net/ffmpeg7arm.zip",
                ArchiveType::SingleBinary,
            )
        } else {
            (
                "https://www.osxexperts.net/ffmpeg7intel.zip",
                ArchiveType::SingleBinary,
            )
        }
    } else if cfg!(target_arch = "aarch64") {
        (
            "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linuxarm64-gpl.tar.xz",
            ArchiveType::TarXz,
        )
    } else {
        (
            "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz",
            ArchiveType::TarXz,
        )
    }
}

async fn extract_zip_ffmpeg(
    data: &[u8],
    bin_dir: &std::path::Path,
    ffmpeg_name: &str,
    ffprobe_name: &str,
) -> anyhow::Result<()> {
    let data = data.to_vec();
    let bin_dir = bin_dir.to_path_buf();
    let ffmpeg_name = ffmpeg_name.to_string();
    let ffprobe_name = ffprobe_name.to_string();

    tokio::task::spawn_blocking(move || {
        let cursor = std::io::Cursor::new(&data);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|e| anyhow!("Failed to open zip: {}", e))?;

        let targets = [ffmpeg_name.as_str(), ffprobe_name.as_str()];

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| anyhow!("Failed to read zip entry: {}", e))?;

            let name = file.name().to_string();
            for target in &targets {
                if name.ends_with(target) {
                    let dest = bin_dir.join(target);
                    let mut buf = Vec::new();
                    std::io::Read::read_to_end(&mut file, &mut buf)?;
                    std::fs::write(&dest, &buf)?;
                    break;
                }
            }
        }

        Ok::<(), anyhow::Error>(())
    })
    .await
    .map_err(|e| anyhow!("Spawn blocking failed: {}", e))??;

    Ok(())
}

async fn extract_tar_xz_ffmpeg(
    data: &[u8],
    bin_dir: &std::path::Path,
    ffmpeg_name: &str,
    ffprobe_name: &str,
) -> anyhow::Result<()> {
    let data = data.to_vec();
    let bin_dir = bin_dir.to_path_buf();
    let ffmpeg_name = ffmpeg_name.to_string();
    let ffprobe_name = ffprobe_name.to_string();

    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let cursor = std::io::Cursor::new(&data);
        let decompressor = xz2::read::XzDecoder::new(cursor);
        let mut archive = tar::Archive::new(decompressor);
        let targets = [ffmpeg_name.as_str(), ffprobe_name.as_str()];

        for entry_result in archive.entries()
            .map_err(|e| anyhow!("Failed to read tar entries: {}", e))?
        {
            let mut entry = entry_result
                .map_err(|e| anyhow!("Failed to read tar entry: {}", e))?;
            let path = entry.path()
                .map_err(|e| anyhow!("Failed to read entry path: {}", e))?;
            let file_name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            for target in &targets {
                if file_name == *target {
                    let dest = bin_dir.join(target);
                    let mut buf = Vec::new();
                    entry.read_to_end(&mut buf)?;
                    std::fs::write(&dest, &buf)?;
                    break;
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .await
    .map_err(|e| anyhow!("Spawn blocking failed: {}", e))??;
    Ok(())
}

// --- aria2c ---

pub async fn ensure_aria2c() -> Option<PathBuf> {
    if let Some(path) = find_tool("aria2c").await {
        return Some(path);
    }

    // Auto-download only on Windows
    #[cfg(target_os = "windows")]
    {
        match download_aria2c().await {
            Ok(path) => return Some(path),
            Err(e) => {
                tracing::warn!("Failed to download aria2c: {}", e);
            }
        }
    }

    None
}

#[cfg(target_os = "windows")]
async fn download_aria2c() -> anyhow::Result<PathBuf> {
    let bin_dir = managed_bin_dir()
        .ok_or_else(|| anyhow!("Could not determine data directory"))?;
    tokio::fs::create_dir_all(&bin_dir).await?;

    let aria2c_name = bin_name("aria2c");
    let aria2c_target = bin_dir.join(&aria2c_name);

    let url = "https://github.com/aria2/aria2/releases/download/release-1.37.0/aria2-1.37.0-win-64bit-build1.zip";

    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!("Failed to download aria2c: HTTP {}", response.status()));
    }

    let bytes = response.bytes().await?;

    let data = bytes.to_vec();
    let bin_dir_clone = bin_dir.clone();
    let aria2c_name_clone = aria2c_name.clone();

    tokio::task::spawn_blocking(move || {
        let cursor = std::io::Cursor::new(&data);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|e| anyhow!("Failed to open aria2c zip: {}", e))?;

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| anyhow!("Failed to read zip entry: {}", e))?;

            let name = file.name().to_string();
            if name.ends_with(&aria2c_name_clone) {
                let dest = bin_dir_clone.join(&aria2c_name_clone);
                let mut buf = Vec::new();
                std::io::Read::read_to_end(&mut file, &mut buf)?;
                std::fs::write(&dest, &buf)?;
                break;
            }
        }

        Ok::<(), anyhow::Error>(())
    })
    .await
    .map_err(|e| anyhow!("Spawn blocking failed: {}", e))??;

    if !aria2c_target.exists() {
        return Err(anyhow!("aria2c binary not found after extraction"));
    }

    Ok(aria2c_target)
}
