use std::path::PathBuf;
use std::process::Stdio;

use anyhow::anyhow;

pub fn is_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists() || std::env::var("FLATPAK_ID").is_ok()
}

pub(crate) fn managed_bin_dir() -> Option<PathBuf> {
    Some(crate::core::paths::app_data_dir()?.join("bin"))
}

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

    pub fn parse_single_sha256(text: &str) -> Option<String> {
        let first = text.split_whitespace().next()?;
        is_sha256(first).then(|| first.to_lowercase())
    }

    pub fn parse_github_digest(digest: &str) -> Option<String> {
        let hex = digest.trim().strip_prefix("sha256:")?;
        is_sha256(hex).then(|| hex.to_lowercase())
    }

    pub fn verify_sha256(bytes: &[u8], expected: &str, label: &str) -> anyhow::Result<()> {
        let actual = sha256_hex(bytes);
        if actual == expected.to_lowercase() {
            tracing::info!("[integrity] {} verified (sha256 matches)", label);
            return Ok(());
        }
        Err(anyhow!(
            "{}: hash does not match: expected {}, got {}. Download discarded.",
            label,
            expected,
            actual
        ))
    }

    pub async fn expected_from_sums_url(
        client: &reqwest::Client,
        sums_url: &str,
        asset: &str,
    ) -> anyhow::Result<String> {
        let response = client
            .get(sums_url)
            .send()
            .await
            .map_err(|e| anyhow!("could not fetch {}: {}", sums_url, e))?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "could not fetch {}: HTTP {}",
                sums_url,
                response.status()
            ));
        }
        let text = response
            .text()
            .await
            .map_err(|e| anyhow!("unreadable body from {}: {}", sums_url, e))?;
        parse_sha256sums(&text, asset)
            .or_else(|| parse_single_sha256(&text))
            .ok_or_else(|| anyhow!("{} is not listed in {}", asset, sums_url))
    }
}

pub mod ytdlp_onedir {
    use anyhow::anyhow;
    use std::path::{Path, PathBuf};

    pub const DIR_NAME: &str = "yt-dlp_onedir";
    const EXE_NAME: &str = "yt-dlp_macos";

    pub fn asset_name() -> Option<&'static str> {
        if cfg!(target_os = "macos") {
            Some("yt-dlp_macos.zip")
        } else {
            None
        }
    }

    pub fn exe_in(dir: &Path) -> PathBuf {
        dir.join(EXE_NAME)
    }

    pub fn is_complete(dir: &Path) -> bool {
        exe_in(dir).is_file() && dir.join("_internal").is_dir()
    }

    fn sibling(target: &Path, suffix: &str) -> PathBuf {
        let name = target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(DIR_NAME);
        target.with_file_name(format!("{name}{suffix}"))
    }

    pub fn install_from_zip(
        bytes: &[u8],
        target: &Path,
        warm: impl FnOnce(&Path) -> anyhow::Result<()>,
    ) -> anyhow::Result<PathBuf> {
        let staged = sibling(target, ".new");
        let old = sibling(target, ".old");
        let _ = std::fs::remove_dir_all(&staged);
        let result = (|| {
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
                .map_err(|e| anyhow!("yt-dlp onedir: invalid zip: {e}"))?;
            archive
                .extract(&staged)
                .map_err(|e| anyhow!("yt-dlp onedir: could not extract: {e}"))?;
            if !is_complete(&staged) {
                return Err(anyhow!(
                    "yt-dlp onedir: unexpected layout (missing {EXE_NAME} or _internal)"
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(exe_in(&staged), std::fs::Permissions::from_mode(0o755))?;
            }
            warm(&exe_in(&staged))
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_dir_all(&staged);
            return Err(e);
        }
        let _ = std::fs::remove_dir_all(&old);
        let had_previous = target.exists();
        if had_previous {
            std::fs::rename(target, &old)
                .map_err(|e| anyhow!("yt-dlp onedir: could not move the previous one: {e}"))?;
        }
        if let Err(e) = std::fs::rename(&staged, target) {
            if had_previous {
                let _ = std::fs::rename(&old, target);
            }
            let _ = std::fs::remove_dir_all(&staged);
            return Err(anyhow!("yt-dlp onedir: could not install: {e}"));
        }
        let exe = exe_in(target);
        if let Ok(f) = std::fs::File::open(&exe) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        Ok(exe)
    }
}

pub fn bin_name(tool: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{}.exe", tool)
    } else {
        tool.to_string()
    }
}

pub fn has_ffmpeg_and_ffprobe(dir: &std::path::Path) -> bool {
    dir.join(bin_name("ffmpeg")).is_file() && dir.join(bin_name("ffprobe")).is_file()
}

pub fn ffmpeg_location(candidates: &[PathBuf]) -> Option<&std::path::Path> {
    candidates
        .iter()
        .find(|dir| has_ffmpeg_and_ffprobe(dir))
        .or_else(|| candidates.first())
        .map(PathBuf::as_path)
}

fn override_name(tool: &str) -> &str {
    match tool {
        "ffmpeg" => "FFmpeg",
        "yt-dlp" => "yt-dlp",
        other => other,
    }
}

pub async fn find_tool(tool: &str) -> Option<PathBuf> {
    find_tool_with_source(tool).await.map(|(path, _)| path)
}

pub fn worker_mode() -> bool {
    std::env::var_os("OMNIGET_WORKER_MODE").is_some_and(|v| v == "1")
}
pub fn worker_tool(tool: &str) -> Option<PathBuf> {
    let key = match tool {
        "yt-dlp" => "OMNIGET_WORKER_YTDLP",
        "ffmpeg" => "OMNIGET_WORKER_FFMPEG",
        "ffprobe" => "OMNIGET_WORKER_FFPROBE",
        "node" => "OMNIGET_WORKER_NODE",
        "deno" => "OMNIGET_WORKER_DENO",
        _ => return None,
    };
    let path = PathBuf::from(std::env::var_os(key)?);
    (path.is_absolute() && path.is_file()).then_some(path)
}

pub async fn find_tool_with_source(tool: &str) -> Option<(PathBuf, &'static str)> {
    if worker_mode() {
        return worker_tool(tool).map(|p| (p, "worker-pinned"));
    }
    if let Some(custom) = crate::core::binary_overrides::get(override_name(tool)) {
        return Some((custom, "custom"));
    }
    if tool == "yt-dlp" {
        if let Some(path) = crate::core::ytdlp::find_ytdlp_cached().await {
            let managed = managed_bin_dir()
                .map(|d| path.starts_with(&d))
                .unwrap_or(false);
            let source = if cfg!(target_os = "linux") && path.starts_with("/app/bin") {
                "flatpak"
            } else if managed {
                "managed"
            } else {
                "system"
            };
            return Some((path, source));
        }
    }
    let name = bin_name(tool);
    let version_flag = version_flag_for(tool);

    #[cfg(target_os = "linux")]
    {
        let flatpak_path = PathBuf::from("/app/bin").join(&name);
        if flatpak_path.exists() {
            return Some((flatpak_path, "flatpak"));
        }
    }

    if let Some(managed_path) = managed_bin_dir().map(|d| d.join(&name)) {
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
                return Some((managed_path, "managed"));
            }
        }
    }

    let result = {
        let name2 = name.clone();
        let vf = version_flag.to_string();
        tokio::task::spawn_blocking(move || {
            crate::core::process::std_command(&name2)
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
        return Some((abs, "system"));
    }

    None
}

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

pub async fn check_version_at_path(path: &std::path::Path, tool: &str) -> Option<String> {
    let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
    let key = (path.to_path_buf(), mtime);
    if let Ok(cache) = version_cache().lock() {
        if let Some(v) = cache.get(&key) {
            return Some(v.clone());
        }
    }
    let version = check_version_at_path_uncached(path, tool).await?;
    if let Ok(mut cache) = version_cache().lock() {
        cache.insert(key, version.clone());
    }
    Some(version)
}

type VersionKey = (PathBuf, Option<std::time::SystemTime>);

fn version_cache() -> &'static std::sync::Mutex<std::collections::HashMap<VersionKey, String>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<VersionKey, String>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

async fn check_version_at_path_uncached(path: &std::path::Path, tool: &str) -> Option<String> {
    let version_flag = version_flag_for(tool);
    let path = path.to_path_buf();
    let vf = version_flag.to_string();
    let output = tokio::task::spawn_blocking(move || {
        crate::core::ytdlp::ytdlp_std_command(&path)
            .arg(&vf)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    })
    .await
    .ok()?
    .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next().unwrap_or("");

    if tool == "ffmpeg" || tool == "ffprobe" {
        first_line.split_whitespace().nth(2).map(|s| s.to_string())
    } else if tool == "aria2c" {
        first_line.split_whitespace().nth(2).map(|s| s.to_string())
    } else {
        Some(first_line.trim().to_string())
    }
}

pub async fn check_version(tool: &str) -> Option<String> {
    let _timer_start = std::time::Instant::now();
    let path = find_tool(tool).await?;
    let result = check_version_at_path(&path, tool).await;
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
    if worker_mode() {
        return worker_tool("ffmpeg")
            .ok_or_else(|| anyhow::anyhow!("WORKER_DEPENDENCY_UNAVAILABLE"));
    }
    if !is_flatpak() {
        let managed_complete = managed_bin_dir().is_some_and(|d| has_ffmpeg_and_ffprobe(&d));
        if !managed_complete {
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
    install_ffmpeg_into(&bin_dir).await
}

fn staged_ffmpeg(bin_dir: &std::path::Path, name: &str) -> PathBuf {
    bin_dir.join(format!("{}.new", name))
}

async fn prepare_staged_binary(path: PathBuf) -> anyhow::Result<()> {
    check_staged_binary(path, "-version").await
}

fn github_release_asset(url: &str) -> Option<(String, Option<String>, String)> {
    let parts: Vec<&str> = url
        .strip_prefix("https://github.com/")?
        .split('/')
        .collect();
    match parts.as_slice() {
        [owner, repo, "releases", "latest", "download", asset] => {
            Some((format!("{owner}/{repo}"), None, asset.to_string()))
        }
        [owner, repo, "releases", "download", tag, asset] => Some((
            format!("{owner}/{repo}"),
            Some(tag.to_string()),
            asset.to_string(),
        )),
        _ => None,
    }
}

async fn download_github_release_asset(url: &str, id: &str) -> anyhow::Result<Vec<u8>> {
    let (repo, tag, name) =
        github_release_asset(url).ok_or_else(|| anyhow!("{url} is not a GitHub release asset"))?;
    let client = crate::core::tools::github::client()?;
    let asset =
        crate::core::tools::github::asset(&client, &repo, tag.as_deref(), |n| n == name).await?;
    crate::core::tools::github::download(
        &client,
        &asset,
        false,
        &crate::core::tools::noop_progress(),
        id,
    )
    .await
}

async fn install_staged_binary(
    staged: PathBuf,
    target: &std::path::Path,
    version_arg: &'static str,
) -> anyhow::Result<()> {
    let installed = match check_staged_binary(staged.clone(), version_arg).await {
        Ok(()) => replace_managed_binary(&staged, target),
        Err(e) => Err(e),
    };
    let _ = std::fs::remove_file(&staged);
    installed
}

async fn check_staged_binary(path: PathBuf, version_arg: &'static str) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| anyhow!("Could not mark {} as executable: {e}", path.display()))?;
        }
        #[cfg(target_os = "macos")]
        {
            let _ = crate::core::process::std_command("xattr")
                .args(["-d", "com.apple.quarantine"])
                .arg(&path)
                .output();
        }
        let status = crate::core::process::std_command(&path)
            .arg(version_arg)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| anyhow!("{} does not run on this computer: {e}", path.display()))?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow!(
                "{} does not run on this computer ({status})",
                path.display()
            ))
        }
    })
    .await
    .map_err(|e| anyhow!("spawn_blocking failed: {}", e))?
}

async fn install_ffmpeg_into(bin_dir: &std::path::Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(bin_dir)?;
    let ffmpeg_name = bin_name("ffmpeg");
    let ffprobe_name = bin_name("ffprobe");
    let staged: Vec<PathBuf> = [&ffmpeg_name, &ffprobe_name]
        .iter()
        .map(|n| staged_ffmpeg(bin_dir, n))
        .collect();
    let result = stage_ffmpeg(bin_dir, &ffmpeg_name, &ffprobe_name).await;
    if result.is_ok() {
        for path in &staged {
            if !path.exists() {
                continue;
            }
            let name = path
                .file_stem()
                .map(PathBuf::from)
                .ok_or_else(|| anyhow!("staged file without a name"))?;
            replace_managed_binary(path, &bin_dir.join(name))?;
        }
    }
    for path in &staged {
        let _ = std::fs::remove_file(path);
    }
    result?;
    let ffmpeg_target = bin_dir.join(&ffmpeg_name);
    tracing::info!("FFmpeg installed to {}", ffmpeg_target.display());
    Ok(ffmpeg_target)
}

async fn stage_ffmpeg(
    bin_dir: &std::path::Path,
    ffmpeg_name: &str,
    ffprobe_name: &str,
) -> anyhow::Result<()> {
    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(600))
        .build()?;
    let temp_path = bin_dir.join(".ffmpeg_download.tmp");

    for (url, archive_type) in ffmpeg_download_urls() {
        tracing::info!("Downloading FFmpeg component from {}", url);
        let data = if github_release_asset(url).is_some() {
            download_github_release_asset(url, "ffmpeg").await?
        } else {
            let response = client
                .get(url)
                .send()
                .await
                .map_err(|e| anyhow!("Could not reach {url}: {e}"))?;
            if !response.status().is_success() {
                return Err(anyhow!(
                    "Failed to download FFmpeg from {}: HTTP {}",
                    url,
                    response.status()
                ));
            }
            let final_url = response.url().clone();
            let data = response
                .bytes()
                .await
                .map_err(|e| anyhow!("The download from {url} was cut off: {e}"))?
                .to_vec();
            let asset = final_url
                .path_segments()
                .and_then(|mut p| p.next_back())
                .unwrap_or("")
                .to_string();
            let sums_url = format!("{final_url}.sha256");
            let expected = integrity::expected_from_sums_url(&client, &sums_url, &asset)
                .await
                .map_err(|e| anyhow!("FFmpeg: could not check the download: {e}"))?;
            integrity::verify_sha256(&data, &expected, "FFmpeg")?;
            data
        };
        if data.len() < 1_000_000 {
            return Err(anyhow!(
                "Downloaded file from {} is too small ({}B) — likely an error page",
                url,
                data.len()
            ));
        }
        let temp_clone = temp_path.clone();
        tokio::task::spawn_blocking(move || std::fs::write(&temp_clone, &data))
            .await
            .map_err(|e| anyhow!("spawn_blocking failed: {}", e))??;

        let extracted = match archive_type {
            ArchiveType::Zip => {
                extract_zip_ffmpeg(&temp_path, bin_dir, ffmpeg_name, ffprobe_name).await
            }
            ArchiveType::TarXz => {
                extract_tar_xz_ffmpeg(&temp_path, bin_dir, ffmpeg_name, ffprobe_name).await
            }
        };
        let _ = std::fs::remove_file(&temp_path);
        extracted?;
    }

    let staged_main = staged_ffmpeg(bin_dir, ffmpeg_name);
    if !staged_main.exists() {
        return Err(anyhow!("FFmpeg binary not found after extraction"));
    }
    prepare_staged_binary(staged_main).await?;
    let staged_probe = staged_ffmpeg(bin_dir, ffprobe_name);
    if !staged_probe.exists() {
        return Err(anyhow!("ffprobe binary not found after extraction"));
    }
    prepare_staged_binary(staged_probe).await
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
        // WHY: evermeet.cx answers 410 on its API since it removed the endpoint and only ever built for Intel.
        if cfg!(target_arch = "aarch64") {
            vec![
                (
                    "https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/ffmpeg.zip",
                    ArchiveType::Zip,
                ),
                (
                    "https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/ffprobe.zip",
                    ArchiveType::Zip,
                ),
            ]
        } else {
            vec![
                (
                    "https://ffmpeg.martin-riedl.de/redirect/latest/macos/amd64/release/ffmpeg.zip",
                    ArchiveType::Zip,
                ),
                (
                    "https://ffmpeg.martin-riedl.de/redirect/latest/macos/amd64/release/ffprobe.zip",
                    ArchiveType::Zip,
                ),
            ]
        }
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

            let name = entry
                .name()
                .map_err(|e| anyhow!("Failed to read zip entry: {}", e))?
                .to_string();
            let file_name = name.rsplit(['/', '\\']).next().unwrap_or("");
            if name.contains("__MACOSX") {
                continue;
            }
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

pub async fn ensure_js_runtime() -> Option<PathBuf> {
    if worker_mode() {
        return worker_tool("deno").or_else(|| worker_tool("node"));
    }
    for tool in &["deno", "node", "bun"] {
        if let Some(path) = find_tool(tool).await {
            return Some(path);
        }
    }

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

    let data = download_github_release_asset(url, "deno").await?;
    let staged = bin_dir.join(format!("{}.new", deno_name));
    let staged_clone = staged.clone();
    let deno_name_clone = deno_name.clone();

    let extracted = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let cursor = std::io::Cursor::new(&data);
        let mut archive =
            zip::ZipArchive::new(cursor).map_err(|e| anyhow!("Failed to open Deno zip: {}", e))?;

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| anyhow!("Failed to read zip entry: {}", e))?;

            let name = file
                .name()
                .map_err(|e| anyhow!("Failed to read zip entry: {}", e))?
                .to_string();
            if name.ends_with(&deno_name_clone) || name == "deno" || name == "deno.exe" {
                let mut out = std::fs::File::create(&staged_clone)?;
                std::io::copy(&mut file, &mut out)?;
                out.sync_all()?;
                return Ok(());
            }
        }

        Err(anyhow!("Deno binary not found after extraction"))
    })
    .await
    .map_err(|e| anyhow!("Spawn blocking failed: {}", e))?;
    if let Err(e) = extracted {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }

    install_staged_binary(staged, &deno_target, "--version").await?;

    tracing::info!("Deno installed to {}", deno_target.display());
    Ok(deno_target)
}

pub async fn ensure_gallerydl() -> Option<PathBuf> {
    if worker_mode() {
        return None;
    }
    if let Some(path) = find_tool("gallery-dl").await {
        return Some(path);
    }

    match download_gallerydl().await {
        Ok(path) => return Some(path),
        Err(e) => tracing::warn!("Failed to download gallery-dl: {}", e),
    }

    None
}

async fn download_gallerydl() -> anyhow::Result<PathBuf> {
    let bin_dir = managed_bin_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    std::fs::create_dir_all(&bin_dir)?;

    let target = bin_dir.join(bin_name("gallery-dl"));

    #[cfg(target_os = "windows")]
    let url = "https://github.com/gdl-org/builds/releases/latest/download/gallery-dl_windows.exe";
    #[cfg(target_os = "linux")]
    let url = "https://github.com/gdl-org/builds/releases/latest/download/gallery-dl_linux";
    #[cfg(target_os = "macos")]
    let url = "https://github.com/gdl-org/builds/releases/latest/download/gallery-dl_macos";
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    return Err(anyhow!("gallery-dl has no binary for this system"));

    let data = download_github_release_asset(url, "gallery-dl").await?;
    let staged = bin_dir.join(format!("{}.new", bin_name("gallery-dl")));
    let staged_clone = staged.clone();
    let written = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        use std::io::Write;
        let mut out = std::fs::File::create(&staged_clone)?;
        out.write_all(&data)?;
        out.sync_all()?;
        Ok(())
    })
    .await
    .map_err(|e| anyhow!("Spawn blocking failed: {}", e))?;
    if let Err(e) = written {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }

    install_staged_binary(staged, &target, "--version").await?;

    if !target.exists() {
        return Err(anyhow!("gallery-dl binary not found after download"));
    }

    Ok(target)
}

pub async fn ensure_aria2c() -> Option<PathBuf> {
    if worker_mode() {
        return None;
    }
    if let Some(path) = find_tool("aria2c").await {
        return Some(path);
    }

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
const ARIA2C_WIN64_ZIP: (&str, &str) = (
    "https://github.com/aria2/aria2/releases/download/release-1.37.0/aria2-1.37.0-win-64bit-build1.zip",
    "67d015301eef0b612191212d564c5bb0a14b5b9c4796b76454276a4d28d9b288",
);

#[cfg(target_os = "windows")]
async fn download_aria2c() -> anyhow::Result<PathBuf> {
    let bin_dir = managed_bin_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    std::fs::create_dir_all(&bin_dir)?;

    let aria2c_name = bin_name("aria2c");
    let aria2c_target = bin_dir.join(&aria2c_name);

    let (url, expected_sha256) = ARIA2C_WIN64_ZIP;

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
    integrity::verify_sha256(&bytes, expected_sha256, "aria2c")?;

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

            let name = file
                .name()
                .map_err(|e| anyhow!("Failed to read zip entry: {}", e))?
                .to_string();
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
mod install_tests {
    use super::*;

    #[cfg(unix)]
    fn script(path: &std::path::Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_ffmpeg_that_does_not_run_is_refused_before_touching_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("ffmpeg.new");
        script(&bad, "exit 3");
        let err = prepare_staged_binary(bad).await.unwrap_err().to_string();
        assert!(err.contains("does not run"), "{err}");
    }

    fn folder_with(root: &std::path::Path, name: &str, tools: &[&str]) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for tool in tools {
            std::fs::write(dir.join(bin_name(tool)), b"x").unwrap();
        }
        dir
    }

    #[test]
    fn a_folder_with_ffmpeg_but_no_ffprobe_is_passed_over_for_one_that_has_both() {
        let root = tempfile::tempdir().unwrap();
        let lone = folder_with(root.path(), "lone", &["ffmpeg"]);
        let complete = folder_with(root.path(), "complete", &["ffmpeg", "ffprobe"]);
        assert!(!has_ffmpeg_and_ffprobe(&lone));
        assert!(has_ffmpeg_and_ffprobe(&complete));
        let candidates = [lone, complete.clone()];
        assert_eq!(ffmpeg_location(&candidates), Some(complete.as_path()));
    }

    #[test]
    fn the_first_ffmpeg_folder_is_kept_when_none_has_ffprobe() {
        let root = tempfile::tempdir().unwrap();
        let first = folder_with(root.path(), "first", &["ffmpeg"]);
        let second = folder_with(root.path(), "second", &["ffmpeg"]);
        let candidates = [first.clone(), second];
        assert_eq!(ffmpeg_location(&candidates), Some(first.as_path()));
        assert_eq!(ffmpeg_location(&[]), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_staged_tool_that_does_not_run_never_replaces_the_installed_one() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("tool");
        let staged = dir.path().join("tool.new");
        std::fs::write(&target, b"old").unwrap();
        script(&staged, "exit 3");
        assert!(install_staged_binary(staged.clone(), &target, "--version")
            .await
            .is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(!staged.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_staged_tool_that_runs_is_moved_into_place() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("tool");
        let staged = dir.path().join("tool.new");
        script(&staged, "exit 0");
        install_staged_binary(staged.clone(), &target, "--version")
            .await
            .unwrap();
        assert!(target.exists());
        assert!(!staged.exists());
    }

    #[test]
    fn release_urls_are_resolved_to_the_repo_tag_and_asset_to_verify() {
        assert_eq!(
            github_release_asset(
                "https://github.com/denoland/deno/releases/latest/download/deno-aarch64-apple-darwin.zip"
            ),
            Some((
                "denoland/deno".to_string(),
                None,
                "deno-aarch64-apple-darwin.zip".to_string()
            ))
        );
        assert_eq!(
            github_release_asset(
                "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
            ),
            Some((
                "BtbN/FFmpeg-Builds".to_string(),
                Some("latest".to_string()),
                "ffmpeg-master-latest-win64-gpl.zip".to_string()
            ))
        );
        assert_eq!(
            github_release_asset(
                "https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/ffmpeg.zip"
            ),
            None
        );
        assert_eq!(
            github_release_asset("https://github.com/gdl-org/builds/releases"),
            None
        );
    }

    #[test]
    fn replacing_a_binary_leaves_only_the_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("tool");
        let staged = dir.path().join("tool.new");
        std::fs::write(&target, b"old").unwrap();
        std::fs::write(&staged, b"new").unwrap();
        replace_managed_binary(&staged, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!staged.exists());
    }

    #[test]
    fn failed_replace_keeps_the_working_binary() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("tool");
        std::fs::write(&target, b"old").unwrap();
        assert!(replace_managed_binary(&dir.path().join("missing.new"), &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
    }

    #[tokio::test]
    async fn zip_resource_forks_do_not_overwrite_the_real_binary() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("a.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        w.start_file("ffmpeg", opts).unwrap();
        w.write_all(b"real").unwrap();
        w.start_file("__MACOSX/._ffmpeg", opts).unwrap();
        w.write_all(b"fork").unwrap();
        w.finish().unwrap();
        extract_zip_ffmpeg(&zip_path, dir.path(), "ffmpeg", "ffprobe")
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("ffmpeg.new")).unwrap(),
            b"real"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_ffmpeg_download_matches_this_chip_and_avoids_the_removed_api() {
        let urls: Vec<&str> = ffmpeg_download_urls().into_iter().map(|(u, _)| u).collect();
        assert_eq!(urls.len(), 2);
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "amd64"
        };
        assert!(urls
            .iter()
            .all(|u| u.contains(arch) && !u.contains("evermeet")));
    }

    #[tokio::test]
    #[ignore = "downloads FFmpeg from the network"]
    async fn ffmpeg_installs_and_runs_on_this_machine() {
        let dir = tempfile::tempdir().unwrap();
        let installed = install_ffmpeg_into(dir.path()).await.unwrap();
        let out = std::process::Command::new(&installed)
            .arg("-version")
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(dir.path().join(bin_name("ffprobe")).exists());
        assert!(!dir
            .path()
            .join(format!("{}.new", bin_name("ffmpeg")))
            .exists());
    }
}
