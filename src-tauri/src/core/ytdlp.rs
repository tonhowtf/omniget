use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::anyhow;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::models::media::{DownloadResult, FormatInfo};

static YTDLP_UPDATING: AtomicBool = AtomicBool::new(false);
static YTDLP_PATH_CACHE: tokio::sync::OnceCell<Option<PathBuf>> = tokio::sync::OnceCell::const_new();
static FFMPEG_LOCATION_CACHE: tokio::sync::OnceCell<Option<String>> = tokio::sync::OnceCell::const_new();
static COOKIES_BROWSER_CACHE: tokio::sync::OnceCell<Option<String>> = tokio::sync::OnceCell::const_new();
static RATE_LIMIT_429_COUNT: AtomicU64 = AtomicU64::new(0);

struct YtRateLimiter {
    semaphore: tokio::sync::Semaphore,
    last_request_ns: AtomicU64,
}

impl YtRateLimiter {
    async fn acquire(&self) {
        let _permit = self.semaphore.acquire().await.unwrap_or_else(|_| panic!("semaphore closed"));
        let min_interval_ns = 500_000_000u64;
        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let last = self.last_request_ns.load(Ordering::Relaxed);
        let elapsed = now_ns.saturating_sub(last);
        if elapsed < min_interval_ns {
            let wait_ns = min_interval_ns - elapsed;
            let wait_duration = std::time::Duration::from_nanos(wait_ns);
            tokio::time::sleep(wait_duration).await;
        }
        self.last_request_ns.store(now_ns, Ordering::Relaxed);
    }
}

static YT_RATE_LIMITER: OnceLock<YtRateLimiter> = OnceLock::new();

fn yt_rate_limiter() -> &'static YtRateLimiter {
    YT_RATE_LIMITER.get_or_init(|| YtRateLimiter {
        semaphore: tokio::sync::Semaphore::new(3),
        last_request_ns: AtomicU64::new(0),
    })
}

const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub async fn find_ytdlp() -> Option<PathBuf> {
    let _timer_start = std::time::Instant::now();
    let bin_name = if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };

    #[cfg(target_os = "linux")]
    {
        let flatpak_path = PathBuf::from("/app/bin").join(bin_name);
        if flatpak_path.exists() {
            tracing::info!("[perf] find_ytdlp took {:?}", _timer_start.elapsed());
            return Some(flatpak_path);
        }
    }

    if let Ok(output) = crate::core::process::command(bin_name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
        if output.success() {
            tracing::info!("[perf] find_ytdlp took {:?}", _timer_start.elapsed());
            return Some(PathBuf::from(bin_name));
        }
    }

    let managed = managed_ytdlp_path()?;
    if managed.exists() {
        tracing::info!("[perf] find_ytdlp took {:?}", _timer_start.elapsed());
        return Some(managed);
    }

    tracing::info!("[perf] find_ytdlp took {:?}", _timer_start.elapsed());
    None
}

pub async fn find_ytdlp_cached() -> Option<PathBuf> {
    let _timer_start = std::time::Instant::now();
    let result = YTDLP_PATH_CACHE
        .get_or_init(|| async { find_ytdlp().await })
        .await
        .clone();
    tracing::info!("[perf] find_ytdlp_cached: {:?}", _timer_start.elapsed());
    result
}

fn managed_ytdlp_path() -> Option<PathBuf> {
    let data = dirs::data_dir()?;
    let bin_name = if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };
    Some(data.join("omniget").join("bin").join(bin_name))
}

pub async fn ensure_ytdlp() -> anyhow::Result<PathBuf> {
    let _timer_start = std::time::Instant::now();
    if let Some(path) = find_ytdlp_cached().await {
        let path_clone = path.clone();
        tokio::spawn(async move { check_ytdlp_freshness(&path_clone).await; });
        tracing::info!("[perf] ensure_ytdlp took {:?}", _timer_start.elapsed());
        return Ok(path);
    }

    if crate::core::dependencies::is_flatpak() {
        tracing::info!("[perf] ensure_ytdlp took {:?}", _timer_start.elapsed());
        return Err(anyhow!("yt-dlp not found in Flatpak sandbox"));
    }

    let result = download_ytdlp_binary().await;
    tracing::info!("[perf] ensure_ytdlp took {:?}", _timer_start.elapsed());
    result
}

async fn download_ytdlp_binary() -> anyhow::Result<PathBuf> {
    let target = managed_ytdlp_path()
        .ok_or_else(|| anyhow!("Could not determine data directory"))?;

    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let download_url = if cfg!(target_os = "windows") {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
    } else if cfg!(target_os = "macos") {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos"
    } else if cfg!(target_arch = "aarch64") {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux_aarch64"
    } else {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp"
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let response = client.get(download_url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Failed to download yt-dlp: HTTP {}",
            response.status()
        ));
    }

    let bytes = response.bytes().await?;
    tokio::fs::write(&target, &bytes).await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        tokio::fs::set_permissions(&target, perms).await?;
    }

    Ok(target)
}

async fn check_ytdlp_freshness(path: &Path) {
    if let Some(managed) = managed_ytdlp_path() {
        if path != managed.as_path() {
            return;
        }
    } else {
        return;
    }

    if let Ok(meta) = tokio::fs::metadata(path).await {
        if let Ok(modified) = meta.modified() {
            if let Ok(age) = modified.elapsed() {
                if age > std::time::Duration::from_secs(2 * 24 * 60 * 60) {
                    if YTDLP_UPDATING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
                        return;
                    }
                    tracing::info!("yt-dlp is older than 2 days, updating in background");
                    tokio::spawn(async {
                        match download_ytdlp_binary().await {
                            Ok(_) => tracing::info!("yt-dlp updated successfully"),
                            Err(e) => tracing::warn!("Failed to update yt-dlp: {}", e),
                        }
                        YTDLP_UPDATING.store(false, Ordering::SeqCst);
                    });
                }
            }
        }
    }
}

async fn find_ffmpeg_location() -> Option<String> {
    let _timer_start = std::time::Instant::now();
    #[cfg(target_os = "windows")]
    let output = crate::core::process::command("where")
        .arg("ffmpeg.exe")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;

    #[cfg(target_os = "macos")]
    let output = crate::core::process::command("which")
        .arg("ffmpeg")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;

    #[cfg(target_os = "linux")]
    let output = crate::core::process::command("sh")
        .args(["-c", "command -v ffmpeg"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        tracing::info!("[perf] find_ffmpeg_location took {:?}", _timer_start.elapsed());
        return None;
    }

    let path_str = String::from_utf8_lossy(&output.stdout);
    let first_line = path_str.lines().next()?.trim().to_string();
    if first_line.is_empty() {
        tracing::info!("[perf] find_ffmpeg_location took {:?}", _timer_start.elapsed());
        return None;
    }

    let p = PathBuf::from(&first_line);
    let result = if p.exists() {
        p.parent()
            .and_then(|dir| dir.to_str())
            .map(|s| s.to_string())
    } else {
        None
    };
    tracing::info!("[perf] find_ffmpeg_location took {:?}", _timer_start.elapsed());
    result
}

async fn find_ffmpeg_location_cached() -> Option<String> {
    FFMPEG_LOCATION_CACHE
        .get_or_init(|| async { find_ffmpeg_location().await })
        .await
        .clone()
}

fn detect_cookies_browser() -> Option<String> {
    let _timer_start = std::time::Instant::now();
    let result = (|| -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        if let Some(local) = dirs::data_local_dir() {
            if local.join("Google").join("Chrome").join("User Data").is_dir() {
                return Some("chrome".to_string());
            }
            if local
                .join("Microsoft")
                .join("Edge")
                .join("User Data")
                .is_dir()
            {
                return Some("edge".to_string());
            }
            if local
                .join("BraveSoftware")
                .join("Brave-Browser")
                .join("User Data")
                .is_dir()
            {
                return Some("brave".to_string());
            }
        }
        if let Some(roaming) = dirs::data_dir() {
            if roaming
                .join("Mozilla")
                .join("Firefox")
                .join("Profiles")
                .is_dir()
            {
                return Some("firefox".to_string());
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            let support = home.join("Library").join("Application Support");
            if support.join("Google").join("Chrome").is_dir() {
                return Some("chrome".to_string());
            }
            if support.join("Firefox").join("Profiles").is_dir() {
                return Some("firefox".to_string());
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(config) = dirs::config_dir() {
            if config.join("google-chrome").is_dir() {
                return Some("chrome".to_string());
            }
            if config.join("chromium").is_dir() {
                return Some("chromium".to_string());
            }
            if config.join("BraveSoftware").join("Brave-Browser").is_dir() {
                return Some("brave".to_string());
            }
            if config.join("microsoft-edge").is_dir() {
                return Some("edge".to_string());
            }
            if config.join("vivaldi").is_dir() {
                return Some("vivaldi".to_string());
            }
        }
        if let Some(home) = dirs::home_dir() {
            let fp = home.join(".var").join("app");

            if fp.join("com.google.Chrome").join("config").join("google-chrome").is_dir() {
                return Some("chrome".to_string());
            }
            if fp.join("org.chromium.Chromium").join("config").join("chromium").is_dir() {
                return Some("chromium".to_string());
            }
            if fp.join("com.brave.Browser").join("config").join("BraveSoftware").join("Brave-Browser").is_dir() {
                return Some("brave".to_string());
            }

            if home.join("snap").join("chromium").join("common").join("chromium").is_dir() {
                return Some("chromium".to_string());
            }

            if home.join(".mozilla").join("firefox").is_dir() {
                return Some("firefox".to_string());
            }
            if fp.join("org.mozilla.firefox").join(".mozilla").join("firefox").is_dir() {
                return Some("firefox".to_string());
            }
            if home.join("snap").join("firefox").join("common").join(".mozilla").join("firefox").is_dir() {
                return Some("firefox".to_string());
            }
        }
    }
    None
    })();
    tracing::info!("[perf] detect_cookies_browser took {:?}", _timer_start.elapsed());
    result
}

async fn detect_cookies_browser_cached() -> Option<String> {
    COOKIES_BROWSER_CACHE
        .get_or_init(|| async {
            tokio::task::spawn_blocking(detect_cookies_browser)
                .await
                .unwrap_or(None)
        })
        .await
        .clone()
}

fn is_youtube_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("youtube.com") || lower.contains("youtu.be")
}

pub async fn get_video_info(ytdlp: &Path, url: &str) -> anyhow::Result<serde_json::Value> {
    let _timer_start = std::time::Instant::now();

    if is_youtube_url(url) {
        yt_rate_limiter().acquire().await;
    }

    tracing::info!("[yt-dlp] starting info fetch for URL");
    let mut args = vec![
        "--dump-single-json".to_string(),
        "--no-warnings".to_string(),
        "--no-playlist".to_string(),
        "--no-check-certificates".to_string(),
        "--socket-timeout".to_string(),
        "15".to_string(),
        "--retries".to_string(),
        "5".to_string(),
        "--extractor-retries".to_string(),
        "5".to_string(),
        "--retry-sleep".to_string(),
        "exp=1:60".to_string(),
        "--user-agent".to_string(),
        CHROME_UA.to_string(),
        "--skip-download".to_string(),
        "-f".to_string(),
        "b".to_string(),
    ];

    if is_youtube_url(url) {
        args.push("--extractor-args".to_string());
        args.push("youtube:player_client=web".to_string());
        args.push("--sleep-requests".to_string());
        args.push("1".to_string());
    }

    args.push(url.to_string());

    let mut child = crate::core::process::command(ytdlp)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("Failed to run yt-dlp: {}", e))?;
    tracing::info!("[perf] get_video_info: yt-dlp process spawned at {:?}", _timer_start.elapsed());

    let stderr_pipe = child.stderr.take().ok_or_else(|| anyhow!("No stderr"))?;
    let stderr_reader = tokio::spawn(async move {
        let mut buf = String::new();
        let mut lines = BufReader::new(stderr_pipe).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let lower = line.to_lowercase();
            if lower.contains("extracting url") {
                tracing::debug!("[yt-dlp info] Extracting URL");
            } else if lower.contains("downloading") && !lower.contains("download") {
                tracing::debug!("[yt-dlp info] Downloading metadata");
            } else if lower.contains("format") {
                tracing::debug!("[yt-dlp info] Selecting format");
            }
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(180),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| {
        tracing::info!("[perf] get_video_info took {:?}", _timer_start.elapsed());
        anyhow!("Timeout fetching video info (180s)")
    })?
    .map_err(|e| {
        tracing::info!("[perf] get_video_info took {:?}", _timer_start.elapsed());
        anyhow!("Failed to run yt-dlp: {}", e)
    })?;

    let stderr_content = stderr_reader.await.unwrap_or_default();
    tracing::info!("[perf] get_video_info: yt-dlp process exited at {:?}", _timer_start.elapsed());

    if !result.status.success() {
        let stderr = if stderr_content.is_empty() {
            String::from_utf8_lossy(&result.stderr).to_string()
        } else {
            stderr_content
        };

        let stderr_lower = stderr.to_lowercase();
        if stderr_lower.contains("http error 429") {
            RATE_LIMIT_429_COUNT.fetch_add(1, Ordering::Relaxed);
            let sanitized_url = sanitize_log_line(url);
            let player_client = if is_youtube_url(url) { "web" } else { "n/a" };
            tracing::warn!(
                "[yt-429] rate limit in get_video_info: url={} player_client={} retries=3",
                sanitized_url,
                player_client
            );
        }

        tracing::info!("[perf] get_video_info took {:?}", _timer_start.elapsed());
        return Err(anyhow!("yt-dlp failed: {}", stderr.trim()));
    }

    let json: serde_json::Value = serde_json::from_slice(&result.stdout)
        .map_err(|e| anyhow!("yt-dlp returned invalid JSON: {}", e))?;

    tracing::info!("[perf] get_video_info took {:?}", _timer_start.elapsed());
    Ok(json)
}

pub async fn get_playlist_info(
    ytdlp: &Path,
    url: &str,
) -> anyhow::Result<(String, Vec<PlaylistEntry>)> {
    if is_youtube_url(url) {
        yt_rate_limiter().acquire().await;
    }

    let mut args = vec![
        "--flat-playlist".to_string(),
        "--dump-json".to_string(),
        "--no-warnings".to_string(),
        "--socket-timeout".to_string(),
        "30".to_string(),
        "--retries".to_string(),
        "5".to_string(),
        "--extractor-retries".to_string(),
        "5".to_string(),
        "--retry-sleep".to_string(),
        "exp=1:60".to_string(),
        "--user-agent".to_string(),
        CHROME_UA.to_string(),
    ];

    if is_youtube_url(url) {
        args.push("--extractor-args".to_string());
        args.push("youtube:player_client=web,default".to_string());
    }

    args.push(url.to_string());

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        crate::core::process::command(ytdlp)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| anyhow!("Timeout fetching playlist (120s)"))?
    .map_err(|e| anyhow!("Failed to run yt-dlp: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_lower = stderr.to_lowercase();
        if stderr_lower.contains("http error 429") {
            RATE_LIMIT_429_COUNT.fetch_add(1, Ordering::Relaxed);
            let sanitized_url = sanitize_log_line(url);
            let player_client = if is_youtube_url(url) { "web" } else { "n/a" };
            tracing::warn!(
                "[yt-429] rate limit in get_playlist_info: url={} player_client={} retries=3",
                sanitized_url,
                player_client
            );
        }
        return Err(anyhow!("yt-dlp playlist failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    let mut playlist_title = String::new();

    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if playlist_title.is_empty() {
                playlist_title = json
                    .get("playlist_title")
                    .or_else(|| json.get("playlist"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("playlist")
                    .to_string();
            }

            let id = json
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = json
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let url = json
                .get("url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("https://www.youtube.com/watch?v={}", id));
            let duration = json.get("duration").and_then(|v| v.as_f64());

            if !id.is_empty() {
                entries.push(PlaylistEntry {
                    id,
                    title,
                    url,
                    duration,
                });
            }
        }
    }

    Ok((playlist_title, entries))
}

pub struct PlaylistEntry {
    pub id: String,
    pub title: String,
    pub url: String,
    pub duration: Option<f64>,
}

fn parse_destination_line(line: &str) -> Option<String> {
    let line = line.trim();

    if let Some(rest) = line.strip_prefix("[download] Destination:") {
        let path = rest.trim();
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }

    if let Some(rest) = line.strip_prefix("[Merger] Merging formats into \"") {
        let path = rest.trim_end_matches('"');
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }

    None
}

pub async fn write_netscape_cookie_file(
    cookies: &[(String, String)],
    domain: &str,
    path: &Path,
) -> anyhow::Result<()> {
    let mut content = String::from("# Netscape HTTP Cookie File\n");
    for (name, value) in cookies {
        content.push_str(&format!(
            "{}\tTRUE\t/\tTRUE\t0\t{}\t{}\n",
            domain, name, value
        ));
    }
    tokio::fs::write(path, content).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn download_video(
    ytdlp: &Path,
    url: &str,
    output_dir: &Path,
    quality_height: Option<u32>,
    progress: mpsc::Sender<f64>,
    download_mode: Option<&str>,
    format_id: Option<&str>,
    filename_template: Option<&str>,
    referer: Option<&str>,
    cancel_token: CancellationToken,
    cookie_file: Option<&Path>,
    concurrent_fragments: u32,
    download_subtitles: bool,
) -> anyhow::Result<DownloadResult> {
    let _timer_start = std::time::Instant::now();

    if is_youtube_url(url) {
        yt_rate_limiter().acquire().await;
    }

    let _ = progress.send(-1.0).await;

    let mode = download_mode.unwrap_or("auto");
    let is_audio_only = mode == "audio";
    let (ffmpeg_available, ffmpeg_location_result, aria2c_path) = tokio::join!(
        crate::core::ffmpeg::is_ffmpeg_available(),
        find_ffmpeg_location_cached(),
        crate::core::dependencies::ensure_aria2c(),
    );
    let ffmpeg_location = if ffmpeg_available { ffmpeg_location_result } else { None };

    let format_selector = if let Some(fid) = format_id {
        fid.to_string()
    } else {
        match mode {
            "audio" => "ba/b".to_string(),
            "mute" => match quality_height {
                Some(h) if h > 0 => format!("bv*[height<={}]/bv*/b", h),
                _ => "bv*/b".to_string(),
            },
            _ => {
                if ffmpeg_available {
                    match quality_height {
                        Some(h) if h > 0 => format!(
                            "bv*[height<={}]+ba/b[height<={}]/bv*+ba/b/b",
                            h, h
                        ),
                        _ => "bv*+ba/b/b".to_string(),
                    }
                } else {
                    tracing::warn!("[yt-dlp] ffmpeg not available, using single-stream format");
                    match quality_height {
                        Some(h) if h > 0 => format!("b[height<={}]/b", h),
                        _ => "b".to_string(),
                    }
                }
            }
        }
    };

    let dir_len = output_dir.to_string_lossy().len();
    let max_name = if cfg!(target_os = "windows") {
        250_usize.saturating_sub(dir_len).min(200)
    } else {
        200
    };
    let template = filename_template
        .map(|t| t.to_string())
        .unwrap_or_else(|| format!("omniget-%(title).{}s [%(id)s].%(ext)s", max_name));
    let output_template = output_dir
        .join(&template)
        .to_string_lossy()
        .to_string();

    tokio::fs::create_dir_all(output_dir).await?;

    let browser_cookies = if cookie_file.is_none() {
        detect_cookies_browser_cached().await
    } else {
        None
    };
    let use_browser_cookies = false;

    let mut base_args = vec![
        "-f".to_string(),
        format_selector,
    ];

    if let Some(ref_url) = referer {
        base_args.push("--referer".to_string());
        base_args.push(ref_url.to_string());
        base_args.push("--add-headers".to_string());
        base_args.push(format!("Referer:{}", ref_url));
    }

    if let Some(cf) = cookie_file {
        base_args.push("--cookies".to_string());
        base_args.push(cf.to_string_lossy().to_string());
    }

    if let Some(ref loc) = ffmpeg_location {
        base_args.push("--ffmpeg-location".to_string());
        base_args.push(loc.clone());
    }

    let effective_fragments = if is_youtube_url(url) {
        let rate_limit_count = RATE_LIMIT_429_COUNT.load(Ordering::Relaxed);
        let max_frags = if rate_limit_count >= 2 { 2 } else if rate_limit_count > 0 { 4 } else { 8 };
        concurrent_fragments.min(max_frags)
    } else {
        concurrent_fragments
    };
    base_args.push("-N".to_string());
    base_args.push(effective_fragments.to_string());

    if is_youtube_url(url) {
        base_args.push("--extractor-args".to_string());
        base_args.push("youtube:player_client=web".to_string());

        base_args.push("--throttled-rate".to_string());
        base_args.push("500K".to_string());

        base_args.push("--sleep-subtitles".to_string());
        base_args.push("5".to_string());
        base_args.push("--sleep-requests".to_string());
        base_args.push("1".to_string());
    }

    base_args.extend([
        "--buffer-size".to_string(),
        "4M".to_string(),
    ]);
    if !is_youtube_url(url) {
        base_args.extend([
            "--http-chunk-size".to_string(),
            "10M".to_string(),
        ]);
    }

    let mut use_aria2c = aria2c_path.is_some()
        && mode != "audio"
        && cookie_file.is_none()
        && browser_cookies.is_none();

    base_args.extend([
        "--no-check-certificate".to_string(),
        "--no-warnings".to_string(),
        "--no-mtime".to_string(),
        "--user-agent".to_string(),
        CHROME_UA.to_string(),
        "--socket-timeout".to_string(),
        "30".to_string(),
        "--retries".to_string(),
        "5".to_string(),
        "--fragment-retries".to_string(),
        "10".to_string(),
        "--extractor-retries".to_string(),
        "3".to_string(),
        "--file-access-retries".to_string(),
        "3".to_string(),
        "--retry-sleep".to_string(),
        "exp=1:120".to_string(),
        "--trim-filenames".to_string(),
        max_name.to_string(),
        "--no-playlist".to_string(),
        "--newline".to_string(),
        "--progress-template".to_string(),
        "download:%(progress._percent_str)s".to_string(),
        "-o".to_string(),
        output_template,
        "--skip-unavailable-fragments".to_string(),
    ]);

    if cfg!(target_os = "windows") {
        base_args.push("--windows-filenames".to_string());
        base_args.push("--restrict-filenames".to_string());
    }

    let should_download_subs = download_subtitles && RATE_LIMIT_429_COUNT.load(Ordering::Relaxed) < 2;
    let subtitle_args = if should_download_subs {
        vec![
            "--write-sub".to_string(),
            "--write-auto-sub".to_string(),
            "--sub-lang".to_string(),
            "en,pt,es".to_string(),
            "--sub-format".to_string(),
            "best".to_string(),
            "--convert-subs".to_string(),
            "srt".to_string(),
        ]
    } else {
        Vec::new()
    };

    let max_attempts: usize = 3;
    let mut extra_args: Vec<String> = Vec::new();
    let mut last_error = String::new();
    let mut use_subtitles = should_download_subs;
    let mut use_browser_cookies = use_browser_cookies
        || (is_youtube_url(url) && browser_cookies.is_some());

    for attempt in 0..max_attempts {
        tracing::info!("[yt-dlp] download attempt {}/{}", attempt + 1, max_attempts);
        if cancel_token.is_cancelled() {
            tracing::info!("[perf] download_video took {:?}", _timer_start.elapsed());
            anyhow::bail!("Download cancelled");
        }

        if attempt > 0 {
            let wait: u64 = match attempt {
                1 => 3,
                2 => 8,
                _ => 15,
            };
            tracing::info!(
                "[yt-dlp] retry {}/{} after {}s",
                attempt,
                max_attempts - 1,
                wait
            );
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            cleanup_part_files(output_dir).await;
        }

        let mut args = base_args.clone();

        if use_subtitles {
            args.extend(subtitle_args.iter().cloned());
        }

        if use_browser_cookies {
            if let Some(ref browser) = browser_cookies {
                args.push("--cookies-from-browser".to_string());
                args.push(browser.clone());
            }
        }

        if use_aria2c && !use_browser_cookies {
            if let Some(ref a2_path) = aria2c_path {
                let conns = if is_youtube_url(url) { effective_fragments.max(1) } else { effective_fragments.clamp(8, 16) };
                args.push("--downloader".to_string());
                args.push(a2_path.to_string_lossy().to_string());
                args.push("--downloader-args".to_string());
                args.push(format!("aria2c:-x {} -k 1M -j {} --min-split-size=1M --file-allocation=none --optimize-concurrent-downloads=true --auto-file-renaming=false --summary-interval=0", conns, conns));
            }
        }

        args.extend(extra_args.iter().cloned());
        args.push(url.to_string());

        let mut child = crate::core::process::command(ytdlp)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("Failed to start yt-dlp: {}", e))?;
        tracing::info!("[perf] download_video: yt-dlp process spawned at {:?} (attempt {})", _timer_start.elapsed(), attempt + 1);

        let _ = progress.send(-2.0).await;

        let stdout = child.stdout.take().ok_or_else(|| anyhow!("No stdout"))?;
        let stderr_pipe = child.stderr.take().ok_or_else(|| anyhow!("No stderr"))?;

        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        let progress_tx = progress.clone();
        let captured_path: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
        let captured_path_writer = captured_path.clone();

        let line_reader = tokio::spawn(async move {
            let mut phase = 0u32;
            let mut max_reported = 0.0f64;
            let mut first_line_logged = false;
            let mut first_progress_logged = false;
            while let Ok(Some(line)) = lines.next_line().await {
                if !first_line_logged {
                    first_line_logged = true;
                    tracing::info!("[perf] download_video first_byte_time: {:?}", _timer_start.elapsed());
                }
                if let Some(dest) = parse_destination_line(&line) {
                    phase += 1;
                    let mut guard = captured_path_writer.lock().unwrap();
                    *guard = Some(PathBuf::from(dest));
                }
                if line.contains("[Merger]") {
                    if 99.0 > max_reported {
                        max_reported = 99.0;
                        let _ = progress_tx.send(99.0).await;
                    }
                    continue;
                }
                if let Some(pct) = parse_progress_line(&line) {
                    if !first_progress_logged && pct > 0.0 {
                        first_progress_logged = true;
                        tracing::info!("[perf] download_video: first_progress > 0% at {:?}", _timer_start.elapsed());
                    }
                    if is_audio_only {
                        let _ = progress_tx.send(pct).await;
                    } else {
                        let adjusted = if phase <= 1 {
                            pct * 0.5
                        } else {
                            50.0 + pct * 0.5
                        };
                        if adjusted > max_reported {
                            max_reported = adjusted;
                            let _ = progress_tx.send(adjusted).await;
                        }
                    }
                }
            }
        });

        let stderr_reader = tokio::spawn(async move {
            let mut buf = String::new();
            let stderr_buf = BufReader::new(stderr_pipe);
            let mut stderr_lines = stderr_buf.lines();
            while let Ok(Some(line)) = stderr_lines.next_line().await {
                buf.push_str(&line);
                buf.push('\n');
            }
            buf
        });

        let status = tokio::select! {
            s = child.wait() => s.map_err(|e| anyhow!("yt-dlp process failed: {}", e))?,
            _ = cancel_token.cancelled() => {
                let _ = child.kill().await;
                let _ = line_reader.await;
                let _ = stderr_reader.await;
                cleanup_part_files(output_dir).await;
                tracing::info!("[perf] download_video took {:?}", _timer_start.elapsed());
                anyhow::bail!("Download cancelled");
            }
        };

        let _ = line_reader.await;
        let stderr_content = stderr_reader.await.unwrap_or_default();

        if status.success() {
            let _ = progress.send(100.0).await;

            let file_path = {
                let guard = captured_path.lock().unwrap();
                guard.clone()
            };

            let file_path = match file_path {
                Some(p) if p.exists() => p,
                _ => find_downloaded_file(output_dir, url).await?,
            };

            let meta = tokio::fs::metadata(&file_path).await?;
            tracing::info!("[perf] download_video took {:?}", _timer_start.elapsed());
            return Ok(DownloadResult {
                file_path,
                file_size_bytes: meta.len(),
                duration_seconds: 0.0,
            });
        }

        last_error = stderr_content;
        let stderr_lower = last_error.to_lowercase();

        if attempt < max_attempts - 1 {
            if use_aria2c
                && (stderr_lower.contains("aria2")
                    || stderr_lower.contains("external downloader"))
            {
                use_aria2c = false;
                tracing::warn!("[yt-dlp] aria2c failed, retrying with native downloader");
            }

            if stderr_lower.contains("http error 429") {
                RATE_LIMIT_429_COUNT.fetch_add(1, Ordering::Relaxed);
                let sanitized_url = sanitize_log_line(url);
                let player_client = if is_youtube_url(url) { "web" } else { "n/a" };
                let cookies_enabled = use_browser_cookies || cookie_file.is_some();
                tracing::warn!(
                    "[yt-429] rate limit in download_video: url={} attempt={}/{} player_client={} cookies={} aria2c={}",
                    sanitized_url,
                    attempt + 1,
                    max_attempts,
                    player_client,
                    cookies_enabled,
                    use_aria2c
                );
                let base_secs = 10u64 * 2u64.pow(attempt as u32);
                let jitter_secs = (attempt as u64 * 7 + url.len() as u64) % 5;
                let wait_secs = base_secs + jitter_secs;
                tracing::warn!("[yt-dlp] rate limited (429), waiting {}s (base={}s + jitter={}s)", wait_secs, base_secs, jitter_secs);
                tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;

                if stderr_lower.contains("subtitle") && use_subtitles {
                    tracing::warn!("[yt-dlp] 429 on subtitles detected, disabling subtitle download for retry");
                    use_subtitles = false;
                }

                if is_youtube_url(url) {
                    extra_args.retain(|a| a != "--extractor-args" && !a.contains("player_client"));
                    let client = match attempt {
                        0 => "youtube:player_client=web",
                        1 => "youtube:player_client=mweb",
                        _ => "youtube:player_client=ios",
                    };
                    extra_args.push("--extractor-args".to_string());
                    extra_args.push(client.to_string());
                    tracing::warn!("[yt-dlp] 429 detected, rotating player_client to {}", client);
                }
            }

            if stderr_lower.contains("nsig") {
                extra_args.retain(|a| a != "--extractor-args" && !a.contains("player_client"));
                let client = if attempt == 0 {
                    "youtube:player_client=ios"
                } else {
                    "youtube:player_client=mweb"
                };
                extra_args.push("--extractor-args".to_string());
                extra_args.push(client.to_string());
                tracing::warn!("[yt-dlp] nsig error, switching to {}", client);
            }

            if (stderr_lower.contains("http error 403") || stderr_lower.contains("forbidden"))
                && !extra_args.contains(&"--force-ipv4".to_string())
            {
                extra_args.push("--force-ipv4".to_string());
                tracing::warn!("[yt-dlp] 403 forbidden, adding --force-ipv4");
            }

            if stderr_lower.contains("subtitle") && use_subtitles && !stderr_lower.contains("http error 429") {
                tracing::warn!("[yt-dlp] subtitle error detected, disabling subtitles for retry");
                use_subtitles = false;
            }

            if stderr_lower.contains("timed out") || stderr_lower.contains("timeout") {
                tracing::warn!("[yt-dlp] socket timeout on attempt {}", attempt + 1);
            }

            if stderr_lower.contains("certificate") || stderr_lower.contains("ssl") {
                tracing::warn!("[yt-dlp] SSL/certificate error on attempt {}", attempt + 1);
            }

            if (stderr_lower.contains("invalid argument") || stderr_lower.contains("errno 22"))
                && !extra_args.contains(&"--restrict-filenames".to_string())
            {
                extra_args.push("--restrict-filenames".to_string());
                tracing::warn!("[yt-dlp] Errno 22, adding --restrict-filenames");
            }

            if ((stderr_lower.contains("could not") && stderr_lower.contains("cookie"))
                || stderr_lower.contains("cookies-from-browser")
                || stderr_lower.contains("failed to decrypt")
                || stderr_lower.contains("keyring"))
                && use_browser_cookies
            {
                use_browser_cookies = false;
                tracing::warn!(
                    "[yt-dlp] cookies-from-browser failed, retrying without"
                );
            }

            if (stderr_lower.contains("sign in") || stderr_lower.contains("login required"))
                && !use_browser_cookies
                && browser_cookies.is_some()
            {
                use_browser_cookies = true;
                tracing::warn!("[yt-dlp] login required, enabling cookies-from-browser");
            }

            if stderr_lower.contains("requested format") && stderr_lower.contains("not available") {
                tracing::warn!("[yt-dlp] requested format not available, relaxing format selector");
                base_args.retain(|a| a != "-S" && a != "ext:mp4:m4a" && a != "--merge-output-format" && a != "mp4");
                if attempt >= 1 {
                    if let Some(pos) = base_args.iter().position(|a| a == "-f") {
                        if pos + 1 < base_args.len() {
                            base_args[pos + 1] = "b".to_string();
                            tracing::warn!("[yt-dlp] falling back to format selector: b");
                        }
                    }
                }
            }

            let last_line = last_error.lines().last().unwrap_or("unknown error").trim();
            let sanitized = sanitize_log_line(last_line);
            tracing::warn!(
                "[yt-dlp] attempt {}/{} failed: {}",
                attempt + 1,
                max_attempts,
                sanitized
            );
            continue;
        }
    }

    tracing::info!("[perf] download_video took {:?}", _timer_start.elapsed());
    Err(translate_ytdlp_error(&last_error))
}

async fn cleanup_part_files(dir: &Path) {
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".part") || name.ends_with(".ytdl") {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }
}

fn sanitize_log_line(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut remaining = line;
    loop {
        let found = remaining
            .find("https://")
            .or_else(|| remaining.find("http://"));
        match found {
            Some(start) => {
                result.push_str(&remaining[..start]);
                let url_part = &remaining[start..];
                let url_end = url_part
                    .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '>')
                    .unwrap_or(url_part.len());
                let url = &url_part[..url_end];
                if let Some(q) = url.find('?') {
                    result.push_str(&url[..q]);
                } else {
                    result.push_str(url);
                }
                remaining = &url_part[url_end..];
            }
            None => {
                result.push_str(remaining);
                break;
            }
        }
    }
    result
}

fn translate_ytdlp_error(stderr: &str) -> anyhow::Error {
    let lower = stderr.to_lowercase();

    if lower.contains("http error 429") {
        return anyhow!(
            "Server returned error 429 (too many requests). Try again later."
        );
    }
    if lower.contains("http error 403") || lower.contains("forbidden") {
        return anyhow!("Access denied (403). The video may be private or region-restricted.");
    }
    if lower.contains("sign in to confirm") || lower.contains("login required") {
        return anyhow!("Video requires login. Use browser cookies or try another URL.");
    }
    if lower.contains("nsig extraction failed") || lower.contains("nsig") {
        return anyhow!("Video extraction failed. Update yt-dlp or try again.");
    }
    if lower.contains("requested format") && lower.contains("not available") {
        return anyhow!("Requested format is not available. The download will retry with a compatible format.");
    }
    if lower.contains("video unavailable") || lower.contains("not available") {
        return anyhow!("Video unavailable or removed.");
    }
    if lower.contains("private video") {
        return anyhow!("This video is private.");
    }
    if lower.contains("copyright") {
        return anyhow!("Video blocked due to copyright.");
    }
    if lower.contains("geo") && lower.contains("block") {
        return anyhow!("Video restricted in your region.");
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return anyhow!("Connection timed out. Check your internet and try again.");
    }
    if lower.contains("ffmpeg") && (lower.contains("not found") || lower.contains("no such file"))
    {
        return anyhow!("FFmpeg not found. Install FFmpeg to download this format.");
    }
    if lower.contains("unsupported url") || lower.contains("no suitable infojson") {
        return anyhow!("Unsupported URL. Check that the link is correct.");
    }
    if lower.contains("unable to download") && lower.contains("webpage") {
        return anyhow!("Failed to access the page. Check the link and your connection.");
    }
    if lower.contains("is not a valid url") || lower.contains("no video formats") {
        return anyhow!("No video formats found for this link.");
    }

    let last_error_line = stderr
        .lines()
        .rev()
        .find(|l| {
            let t = l.trim().to_lowercase();
            t.starts_with("error:") || t.starts_with("error ")
        })
        .unwrap_or("")
        .trim();

    let msg = if !last_error_line.is_empty() {
        last_error_line
            .strip_prefix("ERROR: ")
            .or_else(|| last_error_line.strip_prefix("ERROR:"))
            .or_else(|| last_error_line.strip_prefix("error: "))
            .unwrap_or(last_error_line)
    } else {
        let trimmed = stderr.trim();
        if trimmed.len() > 300 { &trimmed[..300] } else { trimmed }
    };

    anyhow!("yt-dlp: {}", msg)
}

pub fn get_rate_limit_stats() -> serde_json::Value {
    serde_json::json!({
        "rate_limit_429_count": RATE_LIMIT_429_COUNT.load(Ordering::Relaxed)
    })
}

fn parse_progress_line(line: &str) -> Option<f64> {
    let line = line.trim();
    let pct_str = if let Some(rest) = line.strip_prefix("download:") {
        rest.trim().trim_end_matches('%')
    } else if line.ends_with('%') {
        line.trim_end_matches('%').split_whitespace().last()?
    } else {
        return None;
    };

    pct_str.trim().parse::<f64>().ok()
}

async fn find_downloaded_file(output_dir: &Path, url: &str) -> anyhow::Result<PathBuf> {
    let video_id = extract_id_from_url(url).unwrap_or_default();
    let media_extensions: &[&str] = &[
        "mp4", "mkv", "webm", "m4a", "mp3", "ogg", "opus", "flac", "avi", "mov", "ts", "m4v",
        "3gp", "aac", "wav",
    ];
    let now = std::time::SystemTime::now();
    let recency_limit = std::time::Duration::from_secs(600);

    let mut entries = tokio::fs::read_dir(output_dir).await?;
    let mut candidates: Vec<(PathBuf, std::time::SystemTime, bool)> = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(".part") || name.ends_with(".ytdl") || name.starts_with('.') {
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let is_media = media_extensions
            .iter()
            .any(|&e| ext.eq_ignore_ascii_case(e));
        if !is_media {
            continue;
        }

        if let Ok(meta) = entry.metadata().await {
            if meta.len() == 0 {
                continue;
            }
            if let Ok(modified) = meta.modified() {
                let is_recent = now.duration_since(modified).unwrap_or_default() < recency_limit;
                let matches_id = !video_id.is_empty() && name.contains(&video_id);

                if matches_id || is_recent {
                    candidates.push((path, modified, matches_id));
                }
            }
        }
    }

    candidates.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| b.1.cmp(&a.1)));

    candidates
        .into_iter()
        .next()
        .map(|(p, _, _)| p)
        .ok_or_else(|| anyhow!("Downloaded file not found in {:?}", output_dir))
}

pub fn parse_formats(json: &serde_json::Value) -> Vec<FormatInfo> {
    let formats = match json.get("formats").and_then(|v| v.as_array()) {
        Some(f) => f,
        None => return Vec::new(),
    };

    let mut result = Vec::new();
    for f in formats {
        let format_id = match f.get("format_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => continue,
        };

        let ext = f
            .get("ext")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let width = f.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
        let height = f.get("height").and_then(|v| v.as_u64()).map(|v| v as u32);
        let fps = f.get("fps").and_then(|v| v.as_f64());
        let vcodec = f
            .get("vcodec")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let acodec = f
            .get("acodec")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let filesize = f
            .get("filesize")
            .or_else(|| f.get("filesize_approx"))
            .and_then(|v| v.as_u64());
        let tbr = f.get("tbr").and_then(|v| v.as_f64());
        let format_note = f
            .get("format_note")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let has_video = vcodec.as_deref().map(|v| v != "none").unwrap_or(false);
        let has_audio = acodec.as_deref().map(|v| v != "none").unwrap_or(false);

        let resolution = match (width, height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => Some(format!("{}x{}", w, h)),
            _ => f
                .get("resolution")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        };

        result.push(FormatInfo {
            format_id,
            ext,
            resolution,
            width,
            height,
            fps,
            vcodec,
            acodec,
            filesize,
            tbr,
            has_video,
            has_audio,
            format_note,
        });
    }

    result
}

fn extract_id_from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_lowercase();

    if host.contains("youtu.be") {
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
        return segments.first().map(|s| s.to_string());
    }

    if host.contains("youtube.com") {
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
        if segments.first() == Some(&"shorts") {
            return segments.get(1).map(|s| s.to_string());
        }
        return parsed
            .query_pairs()
            .find(|(k, _)| k == "v")
            .map(|(_, v)| v.to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_progress_download_prefix() {
        assert_eq!(parse_progress_line("download:  45.2%"), Some(45.2));
    }

    #[test]
    fn parse_progress_100_percent() {
        assert_eq!(parse_progress_line("download:100.0%"), Some(100.0));
    }

    #[test]
    fn parse_progress_bare_percent() {
        assert_eq!(parse_progress_line("  92.5%"), Some(92.5));
    }

    #[test]
    fn parse_progress_integer() {
        assert_eq!(parse_progress_line("download:100%"), Some(100.0));
    }

    #[test]
    fn parse_progress_garbage_returns_none() {
        assert_eq!(parse_progress_line("[info] Writing video subtitles"), None);
    }

    #[test]
    fn parse_progress_empty_returns_none() {
        assert_eq!(parse_progress_line(""), None);
    }

    #[test]
    fn parse_destination_standard() {
        assert_eq!(
            parse_destination_line("[download] Destination: /tmp/video.mp4"),
            Some("/tmp/video.mp4".to_string())
        );
    }

    #[test]
    fn parse_destination_merger() {
        assert_eq!(
            parse_destination_line("[Merger] Merging formats into \"/tmp/video.mp4\""),
            Some("/tmp/video.mp4".to_string())
        );
    }

    #[test]
    fn parse_destination_no_match() {
        assert_eq!(parse_destination_line("[download] 100% of 50.0MiB"), None);
    }

    #[test]
    fn parse_destination_empty_path_returns_none() {
        assert_eq!(parse_destination_line("[download] Destination:"), None);
    }

    #[test]
    fn is_youtube_url_standard() {
        assert!(is_youtube_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
    }

    #[test]
    fn is_youtube_url_short() {
        assert!(is_youtube_url("https://youtu.be/dQw4w9WgXcQ"));
    }

    #[test]
    fn is_youtube_url_case_insensitive() {
        assert!(is_youtube_url("https://www.YouTube.com/watch?v=test"));
    }

    #[test]
    fn is_youtube_url_other_site() {
        assert!(!is_youtube_url("https://vimeo.com/123456"));
    }

    #[test]
    fn sanitize_strips_query_params() {
        let input = "Error downloading https://example.com/video?token=secret&key=123 failed";
        let result = sanitize_log_line(input);
        assert_eq!(result, "Error downloading https://example.com/video failed");
    }

    #[test]
    fn sanitize_preserves_url_without_query() {
        let input = "Error downloading https://example.com/video failed";
        let result = sanitize_log_line(input);
        assert_eq!(result, input);
    }

    #[test]
    fn sanitize_multiple_urls() {
        let input = "from https://a.com/x?s=1 to https://b.com/y?t=2 done";
        let result = sanitize_log_line(input);
        assert_eq!(result, "from https://a.com/x to https://b.com/y done");
    }

    #[test]
    fn sanitize_no_urls() {
        let input = "plain error message";
        let result = sanitize_log_line(input);
        assert_eq!(result, input);
    }

    #[test]
    fn extract_id_youtube_standard() {
        assert_eq!(
            extract_id_from_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
    }

    #[test]
    fn extract_id_youtu_be() {
        assert_eq!(
            extract_id_from_url("https://youtu.be/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
    }

    #[test]
    fn extract_id_shorts() {
        assert_eq!(
            extract_id_from_url("https://www.youtube.com/shorts/abc123"),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn extract_id_non_youtube() {
        assert_eq!(extract_id_from_url("https://vimeo.com/123456"), None);
    }

    #[test]
    fn translate_error_429() {
        let err = translate_ytdlp_error("HTTP Error 429: Too Many Requests");
        assert!(err.to_string().contains("429"));
    }

    #[test]
    fn translate_error_403() {
        let err = translate_ytdlp_error("HTTP Error 403: Forbidden");
        assert!(err.to_string().contains("403"));
    }

    #[test]
    fn translate_error_nsig() {
        let err = translate_ytdlp_error("nsig extraction failed");
        assert!(err.to_string().contains("extraction"));
    }

    #[test]
    fn translate_error_unavailable() {
        let err = translate_ytdlp_error("Video unavailable");
        assert!(err.to_string().contains("unavailable"));
    }

    #[test]
    fn translate_error_requested_format() {
        let err = translate_ytdlp_error("ERROR: Requested format is not available. Use --list-formats for a list of available formats");
        assert!(err.to_string().contains("Requested format"));
    }

    #[test]
    fn translate_error_private() {
        let err = translate_ytdlp_error("This is a private video");
        assert!(err.to_string().contains("private"));
    }

    #[test]
    fn translate_error_timeout() {
        let err = translate_ytdlp_error("Connection timed out");
        assert!(err.to_string().contains("timed out"));
    }

    #[test]
    fn translate_error_unknown_falls_through() {
        let err = translate_ytdlp_error("ERROR: some unknown thing happened");
        assert!(err.to_string().contains("yt-dlp"));
    }

    #[test]
    fn parse_formats_empty_json() {
        let json = serde_json::json!({});
        assert!(parse_formats(&json).is_empty());
    }

    #[test]
    fn parse_formats_extracts_fields() {
        let json = serde_json::json!({
            "formats": [
                {
                    "format_id": "22",
                    "ext": "mp4",
                    "width": 1280,
                    "height": 720,
                    "fps": 30.0,
                    "vcodec": "avc1.64001F",
                    "acodec": "mp4a.40.2",
                    "filesize": 50_000_000u64,
                    "tbr": 2500.0,
                    "format_note": "720p"
                }
            ]
        });
        let formats = parse_formats(&json);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].format_id, "22");
        assert_eq!(formats[0].height, Some(720));
        assert!(formats[0].has_video);
        assert!(formats[0].has_audio);
        assert_eq!(formats[0].resolution, Some("1280x720".to_string()));
    }

    #[test]
    fn parse_formats_video_only() {
        let json = serde_json::json!({
            "formats": [
                {
                    "format_id": "137",
                    "ext": "mp4",
                    "width": 1920,
                    "height": 1080,
                    "vcodec": "avc1.640028",
                    "acodec": "none"
                }
            ]
        });
        let formats = parse_formats(&json);
        assert_eq!(formats.len(), 1);
        assert!(formats[0].has_video);
        assert!(!formats[0].has_audio);
    }
}
