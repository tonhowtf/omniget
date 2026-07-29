use std::path::PathBuf;
use std::process::Stdio;

use anyhow::anyhow;

pub fn is_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists() || std::env::var("FLATPAK_ID").is_ok()
}

fn managed_bin_dir() -> Option<PathBuf> {
    Some(crate::core::paths::app_data_dir()?.join("bin"))
}

/// Verificação de integridade dos binários gerenciados.
///
/// Regra: **fail-closed**. Se a origem publica um hash e ele não confere, ou
/// se o hash esperado não pôde ser obtido de uma origem que sabidamente o
/// publica, o binário é descartado.
pub mod integrity {
    use anyhow::anyhow;

    pub fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }

    fn is_sha256(candidate: &str) -> bool {
        candidate.len() == 64 && candidate.bytes().all(|b| b.is_ascii_hexdigit())
    }

    /// Lê um arquivo estilo `sha256sum`: `<64-hex><espaços>[*]<nome>`.
    /// Linha em branco ou malformada é pulada — não interrompe a busca.
    pub fn parse_sha256sums(text: &str, asset: &str) -> Option<String> {
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            let (Some(hash), Some(name)) = (parts.next(), parts.next()) else {
                continue;
            };
            let name = name.trim_start_matches('*');
            if name == asset && is_sha256(hash) {
                return Some(hash.to_lowercase());
            }
        }
        None
    }

    /// Arquivo `.sha256sum` de asset único (o Deno publica um por asset).
    pub fn parse_single_sha256(text: &str) -> Option<String> {
        let first = text.split_whitespace().next()?;
        is_sha256(first).then(|| first.to_lowercase())
    }

    /// `digest` da API de releases do GitHub, no formato `sha256:<hex>`.
    pub fn parse_github_digest(digest: &str) -> Option<String> {
        let hex = digest.trim().strip_prefix("sha256:")?;
        is_sha256(hex).then(|| hex.to_lowercase())
    }

    pub fn verify_sha256(bytes: &[u8], expected: &str, label: &str) -> anyhow::Result<()> {
        let actual = sha256_hex(bytes);
        if actual == expected.to_lowercase() {
            tracing::info!("[integrity] {} verificado (sha256 confere)", label);
            return Ok(());
        }
        Err(anyhow!(
            "{}: hash nao confere — esperado {}, obtido {}. Download descartado.",
            label,
            expected,
            actual
        ))
    }

    /// Busca o hash esperado num arquivo de sums remoto. `Err` quando a origem
    /// publica sums mas não conseguimos obtê-los — o chamador deve abortar.
    pub async fn expected_from_sums_url(
        client: &reqwest::Client,
        sums_url: &str,
        asset: &str,
    ) -> anyhow::Result<String> {
        let response = client
            .get(sums_url)
            .send()
            .await
            .map_err(|e| anyhow!("nao foi possivel buscar {}: {}", sums_url, e))?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "nao foi possivel buscar {}: HTTP {}",
                sums_url,
                response.status()
            ));
        }
        let text = response
            .text()
            .await
            .map_err(|e| anyhow!("corpo ilegivel de {}: {}", sums_url, e))?;
        parse_sha256sums(&text, asset)
            .or_else(|| parse_single_sha256(&text))
            .ok_or_else(|| anyhow!("{} nao esta listado em {}", asset, sums_url))
    }
}

pub fn bin_name(tool: &str) -> String {
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
            tracing::debug!(
                "[perf] find_tool({}) took {:?}",
                tool,
                _timer_start.elapsed()
            );
            return Some(flatpak_path);
        }
    }

    // Check managed bin dir first — managed binaries are known-good.
    let managed = managed_bin_dir().map(|d| d.join(&name));
    if let Some(ref managed_path) = managed {
        if managed_path.exists() {
            let check = {
                let managed = managed_path.clone();
                let vf = version_flag.to_string();
                tokio::task::spawn_blocking(move || {
                    crate::core::process::std_command(&managed)
                        .arg(&vf)
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                        .ok()
                        .filter(|s| s.success())
                })
                .await
                .ok()
                .flatten()
            };

            if check.is_some() {
                tracing::debug!(
                    "[perf] find_tool({}) took {:?}",
                    tool,
                    _timer_start.elapsed()
                );
                return Some(managed_path.clone());
            }
            tracing::warn!(
                "find_tool({}): binary exists at {} but failed to execute",
                tool,
                managed_path.display()
            );
        }
    }

    // Fall back to system PATH. Resolve to an absolute path so callers
    // (e.g. find_ffmpeg_location) can derive the parent directory.
    let result = {
        let name = name.clone();
        let vf = version_flag.to_string();
        tokio::task::spawn_blocking(move || {
            crate::core::process::std_command(&name)
                .arg(&vf)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .ok()
                .filter(|s| s.success())
        })
        .await
        .ok()
        .flatten()
    };

    if result.is_some() {
        let abs = resolve_absolute_path(&name);
        tracing::debug!(
            "[perf] find_tool({}) took {:?}",
            tool,
            _timer_start.elapsed()
        );
        return Some(abs);
    }

    tracing::debug!(
        "[perf] find_tool({}) took {:?}",
        tool,
        _timer_start.elapsed()
    );
    None
}

/// Resolve a bare binary name to its absolute path via `where` (Windows)
/// or `which` (Unix). Returns the original name as fallback.
fn resolve_absolute_path(bin_name: &str) -> PathBuf {
    let finder = if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    };
    if let Ok(output) = crate::core::process::std_command(finder)
        .arg(bin_name)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    {
        if output.status.success() {
            if let Some(line) = String::from_utf8_lossy(&output.stdout).lines().next() {
                let path = line.trim();
                if !path.is_empty() {
                    return PathBuf::from(path);
                }
            }
        }
    }
    PathBuf::from(bin_name)
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
    let output = {
        let path = path.clone();
        let vf = version_flag.to_string();
        tokio::task::spawn_blocking(move || {
            crate::core::process::std_command(&path)
                .arg(&vf)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
        })
        .await
        .ok()?
        .ok()?
    };

    if !output.status.success() {
        tracing::debug!(
            "[perf] check_version({}) took {:?}",
            tool,
            _timer_start.elapsed()
        );
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next().unwrap_or("");

    let result = if tool == "ffmpeg" || tool == "ffprobe" {
        first_line.split_whitespace().nth(2).map(|s| s.to_string())
    } else if tool == "yt-dlp" {
        Some(first_line.trim().to_string())
    } else if tool == "aria2c" {
        first_line.split_whitespace().nth(2).map(|s| s.to_string())
    } else {
        Some(first_line.trim().to_string())
    };

    tracing::debug!(
        "[perf] check_version({}) took {:?}",
        tool,
        _timer_start.elapsed()
    );
    result
}

pub fn replace_managed_binary(
    temp: &std::path::Path,
    target: &std::path::Path,
) -> anyhow::Result<()> {
    if !target.exists() {
        std::fs::rename(temp, target)
            .map_err(|e| anyhow!("Failed to move {} into place: {}", target.display(), e))?;
        return Ok(());
    }

    if cfg!(windows) {
        let file_name = target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("binary")
            .to_string();
        let old = target.with_file_name(format!("{}.old", file_name));
        let _ = std::fs::remove_file(&old);
        if let Err(e) = std::fs::rename(target, &old) {
            let _ = std::fs::remove_file(temp);
            return Err(anyhow!(
                "{} is in use by another process ({}). Wait for active downloads to finish or cancel them, then try again.",
                file_name,
                e
            ));
        }
        if let Err(e) = std::fs::rename(temp, target) {
            let _ = std::fs::rename(&old, target);
            let _ = std::fs::remove_file(temp);
            return Err(anyhow!("Failed to replace {}: {}", file_name, e));
        }
        let _ = std::fs::remove_file(&old);
        Ok(())
    } else {
        std::fs::rename(temp, target)
            .map_err(|e| anyhow!("Failed to replace {}: {}", target.display(), e))?;
        Ok(())
    }
}

pub async fn update_ffmpeg() -> anyhow::Result<PathBuf> {
    if is_flatpak() {
        return Err(anyhow!(
            "FFmpeg is provided by the Flatpak runtime and cannot be updated from inside the app"
        ));
    }
    let path = download_ffmpeg().await?;
    crate::core::ytdlp::reset_ffmpeg_location_cache();
    Ok(path)
}

pub async fn ensure_ffmpeg() -> anyhow::Result<PathBuf> {
    // Always ensure the managed binary exists — the standalone yt-dlp.exe
    // cannot discover system FFmpeg from PATH.
    if !is_flatpak() {
        let managed = managed_bin_dir().map(|d| d.join(bin_name("ffmpeg")));
        if managed.as_ref().map_or(true, |p| !p.exists()) {
            if let Ok(path) = download_ffmpeg().await {
                crate::core::ytdlp::reset_ffmpeg_location_cache();
                return Ok(path);
            }
        }
    }

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
    let bin_dir = managed_bin_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    std::fs::create_dir_all(&bin_dir)?;

    let ffmpeg_name = bin_name("ffmpeg");
    let ffprobe_name = bin_name("ffprobe");
    let ffmpeg_target = bin_dir.join(&ffmpeg_name);

    let downloads = ffmpeg_download_urls();

    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    for (url, archive_type) in downloads {
        tracing::info!("Downloading FFmpeg component from {}", url);
        let response = client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "Failed to download FFmpeg from {}: HTTP {}",
                url,
                response.status()
            ));
        }

        let temp_path = bin_dir.join(".ffmpeg_download.tmp");
        let bytes = response.bytes().await?;
        let data = bytes.to_vec();
        let temp_clone = temp_path.clone();
        tokio::task::spawn_blocking(move || std::fs::write(&temp_clone, &data))
            .await
            .map_err(|e| anyhow!("spawn_blocking failed: {}", e))??;

        let file_size = std::fs::metadata(&temp_path)?.len();
        if file_size < 1_000_000 {
            let _ = std::fs::remove_file(&temp_path);
            return Err(anyhow!(
                "Downloaded file from {} is too small ({}B) — likely an error page",
                url,
                file_size
            ));
        }

        match archive_type {
            ArchiveType::Zip => {
                extract_zip_ffmpeg(&temp_path, &bin_dir, &ffmpeg_name, &ffprobe_name).await?
            }
            ArchiveType::TarXz => {
                extract_tar_xz_ffmpeg(&temp_path, &bin_dir, &ffmpeg_name, &ffprobe_name).await?
            }
        }

        let _ = std::fs::remove_file(&temp_path);
    }

    for name in [&ffmpeg_name, &ffprobe_name] {
        let staged = bin_dir.join(format!("{}.new", name));
        if staged.exists() {
            replace_managed_binary(&staged, &bin_dir.join(name))?;
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        let _ = std::fs::set_permissions(&ffmpeg_target, perms.clone());
        let ffprobe_path = bin_dir.join(&ffprobe_name);
        if ffprobe_path.exists() {
            let _ = std::fs::set_permissions(&ffprobe_path, perms);
        }
    }

    #[cfg(target_os = "macos")]
    {
        let ffmpeg_mac = ffmpeg_target.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || {
            crate::core::process::std_command("xattr")
                .args(["-d", "com.apple.quarantine"])
                .arg(&ffmpeg_mac)
                .output()
        })
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))
        .and_then(|r| r)
        {
            tracing::warn!("Failed to remove quarantine from ffmpeg: {}", e);
        }
        let ffprobe_path = bin_dir.join(&ffprobe_name);
        if ffprobe_path.exists() {
            let ffprobe_mac = ffprobe_path.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || {
                crate::core::process::std_command("xattr")
                    .args(["-d", "com.apple.quarantine"])
                    .arg(&ffprobe_mac)
                    .output()
            })
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))
            .and_then(|r| r)
            {
                tracing::warn!("Failed to remove quarantine from ffprobe: {}", e);
            }
        }
    }

    if !ffmpeg_target.exists() {
        return Err(anyhow!("FFmpeg binary not found after extraction"));
    }

    let verify = {
        let target = ffmpeg_target.clone();
        tokio::task::spawn_blocking(move || {
            crate::core::process::std_command(&target)
                .arg("-version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {}", e))?
    };
    match verify {
        Ok(s) if s.success() => {}
        Ok(s) => {
            return Err(anyhow!(
                "FFmpeg installed but failed to execute (exit code {})",
                s
            ))
        }
        Err(e) => return Err(anyhow!("FFmpeg installed but failed to execute: {}", e)),
    }

    tracing::info!("FFmpeg installed to {}", ffmpeg_target.display());
    Ok(ffmpeg_target)
}

enum ArchiveType {
    Zip,
    TarXz,
}

fn ffmpeg_download_urls() -> Vec<(&'static str, ArchiveType)> {
    if cfg!(target_os = "windows") {
        vec![(
            "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip",
            ArchiveType::Zip,
        )]
    } else if cfg!(target_os = "macos") {
        vec![
            (
                "https://evermeet.cx/ffmpeg/getrelease/zip",
                ArchiveType::Zip,
            ),
            (
                "https://evermeet.cx/ffmpeg/getrelease/ffprobe/zip",
                ArchiveType::Zip,
            ),
        ]
    } else if cfg!(target_arch = "aarch64") {
        vec![(
            "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linuxarm64-gpl.tar.xz",
            ArchiveType::TarXz,
        )]
    } else {
        vec![(
            "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz",
            ArchiveType::TarXz,
        )]
    }
}

async fn extract_zip_ffmpeg(
    archive_path: &std::path::Path,
    bin_dir: &std::path::Path,
    ffmpeg_name: &str,
    ffprobe_name: &str,
) -> anyhow::Result<()> {
    let archive_path = archive_path.to_path_buf();
    let bin_dir = bin_dir.to_path_buf();
    let ffmpeg_name = ffmpeg_name.to_string();
    let ffprobe_name = ffprobe_name.to_string();

    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&archive_path)
            .map_err(|e| anyhow!("Failed to open archive: {}", e))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| anyhow!("Failed to open zip: {}", e))?;

        let targets = [ffmpeg_name.as_str(), ffprobe_name.as_str()];

        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| anyhow!("Failed to read zip entry: {}", e))?;

            let name = entry.name().to_string();
            for target in &targets {
                if name.ends_with(target) {
                    let dest = bin_dir.join(format!("{}.new", target));
                    let mut out = std::fs::File::create(&dest)?;
                    std::io::copy(&mut entry, &mut out)?;
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
    archive_path: &std::path::Path,
    bin_dir: &std::path::Path,
    ffmpeg_name: &str,
    ffprobe_name: &str,
) -> anyhow::Result<()> {
    let archive_path = archive_path.to_path_buf();
    let bin_dir = bin_dir.to_path_buf();
    let ffmpeg_name = ffmpeg_name.to_string();
    let ffprobe_name = ffprobe_name.to_string();

    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&archive_path)
            .map_err(|e| anyhow!("Failed to open archive: {}", e))?;
        let decompressor = xz2::read::XzDecoder::new(file);
        let mut archive = tar::Archive::new(decompressor);
        let targets = [ffmpeg_name.as_str(), ffprobe_name.as_str()];

        for entry_result in archive
            .entries()
            .map_err(|e| anyhow!("Failed to read tar entries: {}", e))?
        {
            let mut entry = entry_result.map_err(|e| anyhow!("Failed to read tar entry: {}", e))?;
            let path = entry
                .path()
                .map_err(|e| anyhow!("Failed to read entry path: {}", e))?;
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            for target in &targets {
                if file_name == *target {
                    let dest = bin_dir.join(format!("{}.new", target));
                    let mut out = std::fs::File::create(&dest)?;
                    std::io::copy(&mut entry, &mut out)?;
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

// --- deno (JS runtime for yt-dlp nsig challenge) ---

/// Ensures a JavaScript runtime is available for yt-dlp's YouTube nsig
/// challenge solver. Checks for any existing runtime first (Node.js, Deno,
/// Bun), then auto-downloads Deno if none is found.
pub async fn ensure_js_runtime() -> Option<PathBuf> {
    // Check system-installed runtimes first.
    for tool in &["deno", "node", "bun"] {
        if let Some(path) = find_tool(tool).await {
            return Some(path);
        }
    }

    // Check well-known install locations on Windows.
    #[cfg(target_os = "windows")]
    {
        let candidates = [
            r"C:\Program Files\nodejs\node.exe",
            r"C:\Program Files (x86)\nodejs\node.exe",
        ];
        for path in &candidates {
            let p = PathBuf::from(path);
            if p.exists() {
                return Some(p);
            }
        }
    }

    // No runtime found — download Deno (recommended by yt-dlp, fast, single binary).
    match download_deno().await {
        Ok(path) => {
            crate::core::ytdlp::reset_js_runtime_cache();
            Some(path)
        }
        Err(e) => {
            tracing::warn!("Failed to download Deno JS runtime: {}", e);
            None
        }
    }
}

async fn download_deno() -> anyhow::Result<PathBuf> {
    let bin_dir = managed_bin_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    std::fs::create_dir_all(&bin_dir)?;

    let deno_name = bin_name("deno");
    let deno_target = bin_dir.join(&deno_name);

    if deno_target.exists() {
        return Ok(deno_target);
    }

    let url = if cfg!(target_os = "windows") {
        "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "https://github.com/denoland/deno/releases/latest/download/deno-aarch64-apple-darwin.zip"
        } else {
            "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-apple-darwin.zip"
        }
    } else if cfg!(target_arch = "aarch64") {
        "https://github.com/denoland/deno/releases/latest/download/deno-aarch64-unknown-linux-gnu.zip"
    } else {
        "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-unknown-linux-gnu.zip"
    };

    tracing::info!("Downloading Deno JS runtime from {}", url);

    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "Failed to download Deno: HTTP {}",
            response.status()
        ));
    }

    let bytes = response.bytes().await?;
    let data = bytes.to_vec();
    let bin_dir_clone = bin_dir.clone();
    let deno_name_clone = deno_name.clone();

    tokio::task::spawn_blocking(move || {
        let cursor = std::io::Cursor::new(&data);
        let mut archive =
            zip::ZipArchive::new(cursor).map_err(|e| anyhow!("Failed to open Deno zip: {}", e))?;

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| anyhow!("Failed to read zip entry: {}", e))?;

            let name = file.name().to_string();
            if name.ends_with(&deno_name_clone) || name == "deno" || name == "deno.exe" {
                let dest = bin_dir_clone.join(&deno_name_clone);
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

    if !deno_target.exists() {
        return Err(anyhow!("Deno binary not found after extraction"));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&deno_target, std::fs::Permissions::from_mode(0o755));
    }

    #[cfg(target_os = "macos")]
    {
        let deno_mac = deno_target.clone();
        let _ = tokio::task::spawn_blocking(move || {
            crate::core::process::std_command("xattr")
                .args(["-d", "com.apple.quarantine"])
                .arg(&deno_mac)
                .output()
        })
        .await;
    }

    tracing::info!("Deno installed to {}", deno_target.display());
    Ok(deno_target)
}

pub async fn ensure_gallerydl() -> Option<PathBuf> {
    if let Some(path) = find_tool("gallery-dl").await {
        return Some(path);
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        match download_gallerydl().await {
            Ok(path) => return Some(path),
            Err(e) => tracing::warn!("Failed to download gallery-dl: {}", e),
        }
    }

    None
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
async fn download_gallerydl() -> anyhow::Result<PathBuf> {
    let bin_dir = managed_bin_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    std::fs::create_dir_all(&bin_dir)?;

    let target = bin_dir.join(bin_name("gallery-dl"));

    #[cfg(target_os = "windows")]
    let url = "https://github.com/mikf/gallery-dl/releases/latest/download/gallery-dl.exe";
    #[cfg(target_os = "linux")]
    let url = "https://github.com/mikf/gallery-dl/releases/latest/download/gallery-dl.bin";

    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(std::time::Duration::from_secs(180))
        .build()?;

    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "Failed to download gallery-dl: HTTP {}",
            response.status()
        ));
    }

    let bytes = response.bytes().await?;
    let data = bytes.to_vec();
    let target_clone = target.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        std::fs::write(&target_clone, &data)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = std::fs::metadata(&target_clone)?.permissions();
            perm.set_mode(0o755);
            std::fs::set_permissions(&target_clone, perm)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| anyhow!("Spawn blocking failed: {}", e))??;

    if !target.exists() {
        return Err(anyhow!("gallery-dl binary not found after download"));
    }

    Ok(target)
}

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
    let bin_dir = managed_bin_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    std::fs::create_dir_all(&bin_dir)?;

    let aria2c_name = bin_name("aria2c");
    let aria2c_target = bin_dir.join(&aria2c_name);

    let url = "https://github.com/aria2/aria2/releases/download/release-1.37.0/aria2-1.37.0-win-64bit-build1.zip";

    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "Failed to download aria2c: HTTP {}",
            response.status()
        ));
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

#[cfg(test)]
mod integrity_tests {
    use super::integrity::*;

    const VAZIO: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn sha256_de_vetor_conhecido() {
        assert_eq!(sha256_hex(b""), VAZIO);
    }

    #[test]
    fn verify_sha256_recusa_binario_adulterado() {
        assert!(verify_sha256(b"", VAZIO, "t").is_ok());
        assert!(verify_sha256(b"", &VAZIO.to_uppercase(), "t").is_ok());
        let erro = verify_sha256(b"binario trocado", VAZIO, "yt-dlp").unwrap_err();
        assert!(erro.to_string().contains("yt-dlp"));
    }

    #[test]
    fn linha_em_branco_nao_interrompe_a_busca_no_sums() {
        // Regressao: a versao anterior usava `?` dentro do laco, entao a
        // primeira linha vazia ou de um token so abortava a funcao inteira e o
        // asset seguinte nunca era encontrado -> instalacao sem verificacao.
        let sums = format!(
            "\n\ncomentario\n{}  yt-dlp.exe\n\n{}  yt-dlp_macos\n",
            VAZIO,
            "1".repeat(64)
        );
        assert_eq!(
            parse_sha256sums(&sums, "yt-dlp.exe").as_deref(),
            Some(VAZIO)
        );
        assert_eq!(
            parse_sha256sums(&sums, "yt-dlp_macos").as_deref(),
            Some("1".repeat(64).as_str())
        );
        assert_eq!(parse_sha256sums(&sums, "ausente"), None);
    }

    #[test]
    fn sums_aceita_marcador_binario_e_recusa_hash_invalido() {
        let bin = format!("{} *ffmpeg.zip\n", VAZIO);
        assert_eq!(parse_sha256sums(&bin, "ffmpeg.zip").as_deref(), Some(VAZIO));
        assert_eq!(parse_sha256sums("abc  yt-dlp.exe\n", "yt-dlp.exe"), None);
        assert_eq!(
            parse_sha256sums(&format!("{}  yt-dlp.exe\n", "z".repeat(64)), "yt-dlp.exe"),
            None
        );
    }

    #[test]
    fn digest_da_api_do_github() {
        assert_eq!(
            parse_github_digest(&format!("sha256:{}", VAZIO)).as_deref(),
            Some(VAZIO)
        );
        assert_eq!(parse_github_digest(VAZIO), None);
        assert_eq!(parse_github_digest("sha512:abc"), None);
    }
}
