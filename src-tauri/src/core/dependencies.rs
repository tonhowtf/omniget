use std::path::PathBuf;
use std::process::Stdio;

use anyhow::anyhow;

fn managed_bin_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("omniget").join("bin"))
}

fn bin_name(tool: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{}.exe", tool)
    } else {
        tool.to_string()
    }
}

pub async fn find_tool(tool: &str) -> Option<PathBuf> {
    let name = bin_name(tool);

    if let Ok(status) = tokio::process::Command::new(&name)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
        if status.success() {
            return Some(PathBuf::from(&name));
        }
    }

    let managed = managed_bin_dir()?.join(&name);
    if managed.exists() {
        return Some(managed);
    }

    None
}

pub async fn check_version(tool: &str) -> Option<String> {
    let path = find_tool(tool).await?;
    let output = tokio::process::Command::new(&path)
        .arg("-version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next().unwrap_or("");

    if tool == "ffmpeg" || tool == "ffprobe" {
        return first_line
            .split_whitespace()
            .nth(2)
            .map(|s| s.to_string());
    }

    if tool == "yt-dlp" {
        return Some(first_line.trim().to_string());
    }

    Some(first_line.trim().to_string())
}

pub async fn ensure_ffmpeg() -> anyhow::Result<PathBuf> {
    if let Some(path) = find_tool("ffmpeg").await {
        return Ok(path);
    }
    download_ffmpeg().await
}

async fn download_ffmpeg() -> anyhow::Result<PathBuf> {
    let bin_dir = managed_bin_dir()
        .ok_or_else(|| anyhow!("Could not determine data directory"))?;
    tokio::fs::create_dir_all(&bin_dir).await?;

    let ffmpeg_name = bin_name("ffmpeg");
    let ffprobe_name = bin_name("ffprobe");
    let ffmpeg_target = bin_dir.join(&ffmpeg_name);

    let (url, archive_type) = ffmpeg_download_url();

    let client = reqwest::Client::builder()
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
    let temp_dir = bin_dir.join(".ffmpeg_extract_tmp");
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await?;

    let temp_archive = temp_dir.join("ffmpeg.tar.xz");
    tokio::fs::write(&temp_archive, data).await?;

    let status = tokio::process::Command::new("tar")
        .args(["xf", &temp_archive.to_string_lossy(), "-C", &temp_dir.to_string_lossy()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|e| anyhow!("Failed to run tar: {}", e))?;

    if !status.success() {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return Err(anyhow!("tar extraction failed"));
    }

    let targets = [ffmpeg_name, ffprobe_name];
    find_and_copy_binaries(&temp_dir, bin_dir, &targets).await?;

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    Ok(())
}

async fn find_and_copy_binaries(
    search_dir: &std::path::Path,
    dest_dir: &std::path::Path,
    targets: &[&str],
) -> anyhow::Result<()> {
    let mut stack = vec![search_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                for target in targets {
                    if name == *target {
                        let dest = dest_dir.join(target);
                        tokio::fs::copy(&path, &dest).await?;
                    }
                }
            }
        }
    }
    Ok(())
}
