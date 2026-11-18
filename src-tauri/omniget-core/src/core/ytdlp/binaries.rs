use super::*;

pub async fn find_ytdlp() -> Option<PathBuf> {
    if crate::core::dependencies::worker_mode() {
        return crate::core::dependencies::worker_tool("yt-dlp");
    }
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
            tracing::debug!("[perf] find_ytdlp took {:?}", _timer_start.elapsed());
            return Some(flatpak_path);
        }
    }

    let onedir = managed_ytdlp_onedir_exe();
    if onedir.is_none() {
        spawn_onedir_install_if_missing();
    }
    let zipapp = zipapp_with_python().await;
    if zipapp.is_none() && !cfg!(target_os = "macos") {
        spawn_zipapp_download_if_missing().await;
    }
    let onefile = managed_ytdlp_path().filter(|p| p.exists());
    let zipapp_imp = match (&onedir, &zipapp, &onefile) {
        (None, Some(zip), Some(_)) => zipapp_impersonates(zip).await,
        _ => false,
    };
    if let Some(picked) =
        crate::core::impersonation::pick_ytdlp(onedir, zipapp, onefile, zipapp_imp)
    {
        tracing::debug!("[perf] find_ytdlp took {:?}", _timer_start.elapsed());
        return Some(picked);
    }

    let bin_name_owned = bin_name.to_string();
    let found = tokio::task::spawn_blocking(move || {
        crate::core::process::std_command(&bin_name_owned)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()
            .filter(|s| s.success())
    })
    .await
    .ok()
    .flatten();

    if found.is_some() {
        let abs = resolve_absolute_path(bin_name);
        tracing::debug!("[perf] find_ytdlp took {:?}", _timer_start.elapsed());
        return Some(abs);
    }

    tracing::debug!("[perf] find_ytdlp took {:?}", _timer_start.elapsed());
    None
}

pub(super) fn resolve_absolute_path(bin_name: &str) -> PathBuf {
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

fn live_cached_path(cache: &Option<Option<PathBuf>>) -> Option<PathBuf> {
    cache.as_ref()?.as_ref().filter(|p| p.exists()).cloned()
}

pub async fn find_ytdlp_cached() -> Option<PathBuf> {
    let cached = YTDLP_PATH_CACHE
        .read()
        .ok()
        .and_then(|cache| live_cached_path(&cache));
    if cached.is_some() {
        return cached;
    }
    let result = find_ytdlp().await;
    if result.is_some() {
        if let Ok(mut cache) = YTDLP_PATH_CACHE.write() {
            *cache = Some(result.clone());
        }
    }
    result
}

pub fn managed_ytdlp_path() -> Option<PathBuf> {
    if let Some(custom) = crate::core::binary_overrides::get("yt-dlp") {
        return Some(custom);
    }
    let data = crate::core::paths::app_data_dir()?;
    let bin_name = if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };
    Some(data.join("bin").join(bin_name))
}

pub fn managed_ytdlp_zipapp_path() -> Option<PathBuf> {
    if crate::core::binary_overrides::get("yt-dlp").is_some() {
        return None;
    }
    let data = crate::core::paths::app_data_dir()?;
    Some(data.join("bin").join("yt-dlp.pyz"))
}

pub fn managed_ytdlp_onedir_dir() -> Option<PathBuf> {
    crate::core::dependencies::ytdlp_onedir::asset_name()?;
    if crate::core::binary_overrides::get("yt-dlp").is_some() {
        return None;
    }
    let data = crate::core::paths::app_data_dir()?;
    Some(
        data.join("bin")
            .join(crate::core::dependencies::ytdlp_onedir::DIR_NAME),
    )
}

pub fn managed_ytdlp_onedir_exe() -> Option<PathBuf> {
    let dir = managed_ytdlp_onedir_dir()?;
    crate::core::dependencies::ytdlp_onedir::is_complete(&dir)
        .then(|| crate::core::dependencies::ytdlp_onedir::exe_in(&dir))
}

pub fn is_ytdlp_zipapp(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pyz"))
        .unwrap_or(false)
}

pub(super) const MIN_PYTHON: (u32, u32) = (3, 10);

pub(super) static PYTHON_CACHE: OnceLock<Option<PathBuf>> = OnceLock::new();

pub(super) fn parse_python_version(out: &str) -> Option<(u32, u32)> {
    let mut it = out.trim().split('.');
    let major = it.next()?.trim().parse().ok()?;
    let minor = it.next()?.trim().parse().ok()?;
    Some((major, minor))
}

pub(super) fn python_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(custom) = std::env::var("OMNIGET_PYTHON") {
        if !custom.trim().is_empty() {
            v.push(PathBuf::from(custom.trim()));
        }
    }
    if cfg!(target_os = "macos") {
        v.push(PathBuf::from("/opt/homebrew/bin/python3"));
        v.push(PathBuf::from("/usr/local/bin/python3"));
    }
    v.push(PathBuf::from("python3"));
    if cfg!(target_os = "windows") {
        v.push(PathBuf::from("python"));
    }
    if cfg!(unix) {
        v.push(PathBuf::from("/usr/bin/python3"));
    }
    v
}

pub(super) fn probe_python(candidate: &Path) -> Option<PathBuf> {
    let out = crate::core::process::std_command(candidate)
        .args([
            "-c",
            "import sys; print('%d.%d' % sys.version_info[:2]); print(sys.executable)",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let version = parse_python_version(lines.next()?)?;
    if version < MIN_PYTHON {
        tracing::debug!(
            "[yt-dlp] python {} is {}.{}, below {}.{}",
            candidate.display(),
            version.0,
            version.1,
            MIN_PYTHON.0,
            MIN_PYTHON.1
        );
        return None;
    }
    let exe = lines.next().map(str::trim).filter(|s| !s.is_empty());
    Some(
        exe.map(PathBuf::from)
            .unwrap_or_else(|| candidate.to_path_buf()),
    )
}

pub async fn find_python() -> Option<PathBuf> {
    if let Some(cached) = PYTHON_CACHE.get() {
        return cached.clone();
    }
    tokio::task::spawn_blocking(python_cached)
        .await
        .ok()
        .flatten()
}

pub(super) fn python_cached() -> Option<PathBuf> {
    PYTHON_CACHE
        .get_or_init(|| {
            let found = python_candidates().iter().find_map(|c| probe_python(c));
            match &found {
                Some(p) => tracing::info!("[yt-dlp] python for zipapp: {}", p.display()),
                None => {
                    tracing::info!("[yt-dlp] no python >= 3.10 found; using the onefile binary")
                }
            }
            found
        })
        .clone()
}

pub(super) fn worker_retry_args(worker: bool) -> Vec<&'static str> {
    if !worker {
        return Vec::new();
    }
    vec![
        "--ignore-config",
        "--no-plugin-dirs",
        "--downloader",
        "native",
        "--retries",
        "3",
        "--fragment-retries",
        "3",
        "--retry-sleep",
        "exp=1:8",
        "--extractor-retries",
        "0",
        "--file-access-retries",
        "0",
        "--abort-on-unavailable-fragments",
    ]
}

pub(super) fn strip_chunk_size(args: &mut Vec<String>) {
    if let Some(at) = args.iter().position(|a| a == "--http-chunk-size") {
        let end = (at + 2).min(args.len());
        args.drain(at..end);
    }
}

pub fn ytdlp_command(ytdlp: &Path) -> tokio::process::Command {
    if is_ytdlp_zipapp(ytdlp) {
        if let Some(py) = python_cached() {
            let mut cmd = crate::core::process::command(py);
            cmd.arg(ytdlp);
            return cmd;
        }
    }
    crate::core::process::command(ytdlp)
}

pub fn ytdlp_std_command(ytdlp: &Path) -> std::process::Command {
    if is_ytdlp_zipapp(ytdlp) {
        if let Some(py) = python_cached() {
            let mut cmd = crate::core::process::std_command(py);
            cmd.arg(ytdlp);
            return cmd;
        }
    }
    crate::core::process::std_command(ytdlp)
}

pub(super) static ZIPAPP_DOWNLOAD_STARTED: AtomicBool = AtomicBool::new(false);

pub(super) async fn spawn_zipapp_download_if_missing() {
    if crate::core::dependencies::is_flatpak() {
        return;
    }
    let Some(zip) = managed_ytdlp_zipapp_path() else {
        return;
    };
    if zip.exists() || find_python().await.is_none() {
        return;
    }
    if ZIPAPP_DOWNLOAD_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("ytdlp-zipapp".into())
        .spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("ytdlp-zipapp runtime");
            rt.block_on(async {
                match download_ytdlp_zipapp().await {
                    Ok(p) => {
                        tracing::info!("[ytdlp] zipapp ready at {}", p.display());
                        reset_ytdlp_cache();
                    }
                    Err(e) => tracing::warn!("[ytdlp] zipapp download failed: {}", e),
                }
            });
        })
        .ok();
}

pub(super) async fn zipapp_with_python() -> Option<PathBuf> {
    let zip = managed_ytdlp_zipapp_path()?;
    if !zip.exists() {
        return None;
    }
    find_python().await.map(|_| zip)
}

pub(super) async fn usable_zipapp() -> Option<PathBuf> {
    let zip = zipapp_with_python().await?;
    zipapp_impersonates(&zip).await.then_some(zip)
}

pub(super) static ZIPAPP_IMPERSONATES: OnceLock<bool> = OnceLock::new();

pub(super) async fn zipapp_impersonates(zip: &Path) -> bool {
    if let Some(v) = ZIPAPP_IMPERSONATES.get() {
        return *v;
    }
    let zip = zip.to_path_buf();
    let found = tokio::task::spawn_blocking(move || {
        ytdlp_std_command(&zip)
            .arg("--list-impersonate-targets")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .map(|o| {
                !crate::core::impersonation::parse_targets(&String::from_utf8_lossy(&o.stdout))
                    .is_empty()
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    if !found {
        tracing::info!("[yt-dlp] zipapp python has no curl_cffi; preferring a managed binary");
    }
    *ZIPAPP_IMPERSONATES.get_or_init(|| found)
}

pub(super) static ONEDIR_INSTALL_STARTED: AtomicBool = AtomicBool::new(false);

pub(super) fn spawn_onedir_install_if_missing() {
    if crate::core::dependencies::is_flatpak() || crate::core::dependencies::worker_mode() {
        return;
    }
    let Some(dir) = managed_ytdlp_onedir_dir() else {
        return;
    };
    if crate::core::dependencies::ytdlp_onedir::is_complete(&dir) {
        return;
    }
    if ONEDIR_INSTALL_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("ytdlp-onedir".into())
        .spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("ytdlp-onedir runtime");
            rt.block_on(async {
                match download_ytdlp_onedir().await {
                    Ok(p) => {
                        tracing::info!("[ytdlp] onedir ready at {}", p.display());
                        reset_ytdlp_cache();
                    }
                    Err(e) => tracing::warn!("[ytdlp] onedir install failed: {}", e),
                }
            });
        })
        .ok();
}

pub(super) fn warm_ytdlp_onedir(exe: &Path) -> anyhow::Result<()> {
    let out = crate::core::process::std_command(exe)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| anyhow!("yt-dlp onedir does not run: {e}"))?;
    if !out.status.success() || parse_ytdlp_version(&String::from_utf8_lossy(&out.stdout)).is_none()
    {
        return Err(anyhow!("yt-dlp onedir: --version failed"));
    }
    Ok(())
}

pub async fn ensure_ytdlp() -> anyhow::Result<PathBuf> {
    if crate::core::dependencies::worker_mode() {
        return crate::core::dependencies::worker_tool("yt-dlp")
            .ok_or_else(|| anyhow!("WORKER_DEPENDENCY_UNAVAILABLE"));
    }
    let _timer_start = std::time::Instant::now();

    if !crate::core::dependencies::is_flatpak() {
        if let Some(dir) = managed_ytdlp_onedir_dir() {
            if !crate::core::dependencies::ytdlp_onedir::is_complete(&dir) {
                let has_other = managed_ytdlp_path().is_some_and(|p| p.exists())
                    || usable_zipapp().await.is_some();
                if has_other {
                    spawn_onedir_install_if_missing();
                } else {
                    tracing::info!("[ytdlp] managed onedir missing, downloading...");
                    match download_ytdlp_onedir().await {
                        Ok(path) => {
                            reset_ytdlp_cache();
                            tracing::debug!(
                                "[perf] ensure_ytdlp took {:?}",
                                _timer_start.elapsed()
                            );
                            return Ok(path);
                        }
                        Err(e) => tracing::warn!(
                            "[ytdlp] failed to install onedir, falling back to onefile: {}",
                            e
                        ),
                    }
                }
            }
        }
    }

    if !crate::core::dependencies::is_flatpak() && !cfg!(target_os = "macos") {
        if let Some(zip) = managed_ytdlp_zipapp_path() {
            if find_python().await.is_some() && !zip.exists() {
                tracing::info!("[ytdlp] managed zipapp missing, downloading...");
                match download_ytdlp_zipapp().await {
                    Ok(path) => {
                        reset_ytdlp_cache();
                        tracing::debug!("[perf] ensure_ytdlp took {:?}", _timer_start.elapsed());
                        return Ok(path);
                    }
                    Err(e) => tracing::warn!(
                        "[ytdlp] failed to download zipapp, falling back to onefile: {}",
                        e
                    ),
                }
            }
        }
    }

    if !crate::core::dependencies::is_flatpak()
        && managed_ytdlp_onedir_exe().is_none()
        && usable_zipapp().await.is_none()
    {
        let managed = managed_ytdlp_path();
        if managed.as_ref().is_none_or(|p| !p.exists()) {
            tracing::info!("[ytdlp] managed binary missing, downloading...");
            match download_ytdlp_binary().await {
                Ok(path) => {
                    reset_ytdlp_cache();
                    std::thread::Builder::new()
                        .name("js-runtime-check".into())
                        .spawn(|| {
                            let rt = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                                .expect("js-runtime runtime");
                            rt.block_on(async {
                                crate::core::dependencies::ensure_js_runtime().await;
                            });
                        })
                        .ok();
                    tracing::debug!("[perf] ensure_ytdlp took {:?}", _timer_start.elapsed());
                    return Ok(path);
                }
                Err(e) => {
                    tracing::warn!(
                        "[ytdlp] failed to download managed binary, falling back to system: {}",
                        e
                    );
                }
            }
        }
    }

    if let Some(path) = find_ytdlp_cached().await {
        let path_clone = path.clone();
        std::thread::Builder::new()
            .name("ytdlp-freshness".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("freshness runtime");
                rt.block_on(async move {
                    check_ytdlp_freshness(&path_clone).await;
                });
            })
            .ok();
        std::thread::Builder::new()
            .name("js-runtime-check".into())
            .spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("js-runtime runtime");
                rt.block_on(async {
                    crate::core::dependencies::ensure_js_runtime().await;
                });
            })
            .ok();
        tracing::debug!("[perf] ensure_ytdlp took {:?}", _timer_start.elapsed());
        return Ok(path);
    }

    if crate::core::dependencies::is_flatpak() {
        tracing::debug!("[perf] ensure_ytdlp took {:?}", _timer_start.elapsed());
        return Err(anyhow!("yt-dlp not found in Flatpak sandbox"));
    }

    let path = download_ytdlp_binary().await?;
    reset_ytdlp_cache();
    std::thread::Builder::new()
        .name("js-runtime-check".into())
        .spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("js-runtime runtime");
            rt.block_on(async {
                crate::core::dependencies::ensure_js_runtime().await;
            });
        })
        .ok();
    tracing::debug!("[perf] ensure_ytdlp took {:?}", _timer_start.elapsed());
    Ok(path)
}

pub async fn update_ytdlp() -> anyhow::Result<PathBuf> {
    if crate::core::dependencies::is_flatpak() {
        return Err(anyhow!(
            "yt-dlp is provided by the Flatpak runtime and cannot be updated from inside the app"
        ));
    }
    if managed_ytdlp_onedir_exe().is_some() {
        let path = download_ytdlp_onedir().await?;
        reset_ytdlp_cache();
        return Ok(path);
    }
    let path = download_ytdlp_binary().await?;
    reset_ytdlp_cache();
    Ok(path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YtdlpChannel {
    Stable,
    Nightly,
}

pub(super) fn ytdlp_channel() -> YtdlpChannel {
    match std::env::var("OMNIGET_YTDLP_CHANNEL") {
        Ok(v) if v.trim().eq_ignore_ascii_case("nightly") => YtdlpChannel::Nightly,
        _ => YtdlpChannel::Stable,
    }
}

pub(super) fn ytdlp_asset_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else if cfg!(target_arch = "aarch64") {
        "yt-dlp_linux_aarch64"
    } else {
        "yt-dlp"
    }
}

pub(super) fn ytdlp_release_base(channel: YtdlpChannel) -> &'static str {
    match channel {
        YtdlpChannel::Stable => "https://github.com/yt-dlp/yt-dlp/releases/latest/download",
        YtdlpChannel::Nightly => {
            "https://github.com/yt-dlp/yt-dlp-nightly-builds/releases/latest/download"
        }
    }
}

pub(super) use crate::core::dependencies::integrity;

pub(super) async fn download_ytdlp_binary() -> anyhow::Result<PathBuf> {
    let target =
        managed_ytdlp_path().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    download_ytdlp_asset(ytdlp_asset_name(), target).await
}

pub(super) const YTDLP_ZIPAPP_ASSET: &str = "yt-dlp";

pub(super) async fn download_ytdlp_zipapp() -> anyhow::Result<PathBuf> {
    let target =
        managed_ytdlp_zipapp_path().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    download_ytdlp_asset(YTDLP_ZIPAPP_ASSET, target).await
}

pub(super) async fn download_ytdlp_onedir() -> anyhow::Result<PathBuf> {
    let asset = crate::core::dependencies::ytdlp_onedir::asset_name()
        .ok_or_else(|| anyhow!("yt-dlp onedir is not published for this platform"))?;
    let dir =
        managed_ytdlp_onedir_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = fetch_verified_ytdlp_asset(asset, 300).await?;
    tokio::task::spawn_blocking(move || {
        crate::core::dependencies::ytdlp_onedir::install_from_zip(&bytes, &dir, warm_ytdlp_onedir)
    })
    .await
    .map_err(|e| anyhow!("spawn_blocking failed: {}", e))?
}

pub(super) async fn fetch_verified_ytdlp_asset(
    asset: &str,
    timeout_secs: u64,
) -> anyhow::Result<Vec<u8>> {
    let channel = ytdlp_channel();
    let base = ytdlp_release_base(channel);
    let download_url = format!("{}/{}", base, asset);
    let sums_url = format!("{}/SHA2-256SUMS", base);

    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()?;

    let response = client.get(&download_url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Failed to download yt-dlp: HTTP {}",
            response.status()
        ));
    }

    let bytes = response.bytes().await?;
    let expected = integrity::expected_from_sums_url(&client, &sums_url, asset)
        .await
        .map_err(|e| anyhow!("yt-dlp: integrity check not possible: {}", e))?;
    integrity::verify_sha256(&bytes, &expected, &format!("yt-dlp ({:?})", channel))?;
    Ok(bytes.to_vec())
}

pub(super) async fn download_ytdlp_asset(asset: &str, target: PathBuf) -> anyhow::Result<PathBuf> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let channel = ytdlp_channel();
    let base = ytdlp_release_base(channel);
    let download_url = format!("{}/{}", base, asset);
    let sums_url = format!("{}/SHA2-256SUMS", base);

    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(600))
        .build()?;

    let response = client
        .get(&download_url)
        .send()
        .await
        .map_err(|e| anyhow!("Failed to reach the yt-dlp download: {e}"))?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Failed to download yt-dlp: HTTP {}",
            response.status()
        ));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| anyhow!("The yt-dlp download was cut off: {e}"))?;

    let expected = integrity::expected_from_sums_url(&client, &sums_url, asset)
        .await
        .map_err(|e| anyhow!("yt-dlp: integrity check not possible: {}", e))?;
    integrity::verify_sha256(&bytes, &expected, &format!("yt-dlp ({:?})", channel))?;

    let temp = target.with_file_name(format!(
        "{}.new",
        target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("yt-dlp")
    ));
    let temp_clone = temp.clone();
    tokio::task::spawn_blocking(move || std::fs::write(&temp_clone, &bytes))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {}", e))??;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&temp, perms)?;
    }

    crate::core::dependencies::replace_managed_binary(&temp, &target)?;

    #[cfg(target_os = "macos")]
    {
        let target_mac = target.clone();
        let _ = tokio::task::spawn_blocking(move || {
            crate::core::process::std_command("xattr")
                .args(["-d", "com.apple.quarantine"])
                .arg(&target_mac)
                .output()
        })
        .await;
    }

    Ok(target)
}

pub(super) async fn latest_channel_version(channel: YtdlpChannel) -> Option<(u32, u32, u32)> {
    let repo = match channel {
        YtdlpChannel::Stable => "yt-dlp/yt-dlp",
        YtdlpChannel::Nightly => "yt-dlp/yt-dlp-nightly-builds",
    };
    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .ok()?;
    let resp = client
        .get(format!("https://github.com/{repo}/releases/latest"))
        .send()
        .await
        .ok()?;
    let tag = resp.url().path().rsplit('/').next()?.to_string();
    parse_ytdlp_version(&tag)
}

pub(super) async fn local_ytdlp_version(path: &Path) -> Option<(u32, u32, u32)> {
    let p = path.to_path_buf();
    let _slot = acquire_ytdlp_slot("local version").await;
    let out = tokio::task::spawn_blocking(move || {
        ytdlp_std_command(&p)
            .arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    })
    .await
    .ok()?
    .ok()?;
    parse_ytdlp_version(&String::from_utf8_lossy(&out.stdout))
}

pub(crate) fn newer_release_available(
    local: Option<(u32, u32, u32)>,
    remote: Option<(u32, u32, u32)>,
) -> bool {
    matches!((local, remote), (Some(l), Some(r)) if r > l)
}

pub(super) async fn check_ytdlp_freshness(path: &Path) {
    let is_managed = managed_ytdlp_path().is_some_and(|m| path == m.as_path())
        || managed_ytdlp_zipapp_path().is_some_and(|z| path == z.as_path())
        || managed_ytdlp_onedir_exe().is_some_and(|o| path == o.as_path());
    if !is_managed {
        return;
    }

    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    let Ok(modified) = meta.modified() else {
        return;
    };
    let Ok(age) = modified.elapsed() else {
        return;
    };
    if age <= std::time::Duration::from_secs(2 * 24 * 60 * 60) {
        return;
    }
    if active_process_pids()
        .lock()
        .map(|m| !m.is_empty())
        .unwrap_or(true)
    {
        return;
    }
    if YTDLP_UPDATING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let path_owned = path.to_path_buf();
    std::thread::Builder::new()
        .name("ytdlp-update".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("ytdlp-update runtime");
            rt.block_on(async {
                let channel = ytdlp_channel();
                let (local, remote) = tokio::join!(
                    local_ytdlp_version(&path_owned),
                    latest_channel_version(channel)
                );
                if newer_release_available(local, remote) {
                    tracing::info!(
                        "yt-dlp {:?} is older than the {:?} release {:?}, updating in background",
                        local,
                        channel,
                        remote
                    );
                    let result = if is_ytdlp_zipapp(&path_owned) {
                        download_ytdlp_zipapp().await
                    } else if managed_ytdlp_onedir_exe().is_some_and(|o| o == path_owned) {
                        download_ytdlp_onedir().await
                    } else {
                        download_ytdlp_binary().await
                    };
                    match result {
                        Ok(_) => tracing::info!("yt-dlp updated successfully"),
                        Err(e) => tracing::warn!("Failed to update yt-dlp: {}", e),
                    }
                } else {
                    tracing::debug!(
                        "yt-dlp {:?} is current for {:?} (release {:?}); not re-downloading",
                        local,
                        channel,
                        remote
                    );
                    if let Ok(f) = std::fs::File::open(&path_owned) {
                        let _ = f.set_modified(std::time::SystemTime::now());
                    }
                }
                YTDLP_UPDATING.store(false, Ordering::SeqCst);
            });
        })
        .ok();
}

pub(super) async fn find_ffmpeg_location() -> Option<String> {
    let _timer_start = std::time::Instant::now();
    let result = if let Some(path) = crate::core::dependencies::find_tool("ffmpeg").await {
        path.parent()
            .and_then(|dir| dir.to_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    } else {
        None
    };
    tracing::debug!(
        "[perf] find_ffmpeg_location took {:?}",
        _timer_start.elapsed()
    );
    result
}

pub(super) async fn find_ffmpeg_location_cached() -> Option<String> {
    if let Ok(cache) = FFMPEG_LOCATION_CACHE.read() {
        if let Some(ref cached) = *cache {
            if let Some(ref dir) = cached {
                let check_path =
                    std::path::Path::new(dir).join(crate::core::dependencies::bin_name("ffmpeg"));
                if check_path.exists() {
                    return cached.clone();
                }
                tracing::warn!("[ffmpeg] cached location no longer valid: {}", dir);
            } else {
                return None;
            }
        }
    }
    let result = find_ffmpeg_location().await;
    if let Ok(mut cache) = FFMPEG_LOCATION_CACHE.write() {
        *cache = Some(result.clone());
    }
    result
}

pub(super) fn per_domain_cookie_file(url: &str) -> Option<std::path::PathBuf> {
    let source = PER_DOMAIN_COOKIE_FN.get()?(url)?;
    if !source.exists() {
        return None;
    }
    let metadata = std::fs::metadata(&source).ok()?;
    let modified = metadata.modified().ok()?;
    if modified.elapsed().unwrap_or_default() >= std::time::Duration::from_secs(604800 * 4) {
        return None;
    }
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("cookies");
    let copy = source.with_file_name(format!("{}-session.txt", stem));
    std::fs::copy(&source, &copy).ok()?;
    Some(copy)
}

pub(super) fn extension_cookie_file() -> Option<std::path::PathBuf> {
    let source = ext_cookie_path();
    if !source.exists() {
        return None;
    }
    let metadata = std::fs::metadata(&source).ok()?;
    let modified = metadata.modified().ok()?;
    if modified.elapsed().unwrap_or_default() >= std::time::Duration::from_secs(604800) {
        return None;
    }
    let copy = source.with_file_name("chrome-extension-cookies-session.txt");
    std::fs::copy(&source, &copy).ok()?;
    Some(copy)
}

pub(super) fn detect_js_runtime() -> Option<String> {
    let runtimes: &[(&str, &str)] = if cfg!(target_os = "windows") {
        &[
            ("node", "node.exe"),
            ("deno", "deno.exe"),
            ("bun", "bun.exe"),
        ]
    } else {
        &[("node", "node"), ("deno", "deno"), ("bun", "bun")]
    };

    for &(runtime, bin) in runtimes {
        let finder = if cfg!(target_os = "windows") {
            "where"
        } else {
            "which"
        };
        if let Ok(output) = crate::core::process::std_command(finder)
            .arg(bin)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
        {
            if output.status.success() {
                if let Some(line) = String::from_utf8_lossy(&output.stdout).lines().next() {
                    let path = line.trim();
                    if !path.is_empty() && std::path::Path::new(path).exists() {
                        return Some(format!("{}:{}", runtime, path));
                    }
                }
            }
        }
    }

    if let Some(bin_dir) = crate::core::paths::app_data_dir().map(|d| d.join("bin")) {
        for &(runtime, bin) in runtimes {
            let managed = bin_dir.join(bin);
            if managed.exists() {
                return Some(format!("{}:{}", runtime, managed.display()));
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let candidates = [
            ("node", r"C:\Program Files\nodejs\node.exe"),
            ("node", r"C:\Program Files (x86)\nodejs\node.exe"),
        ];
        for (runtime, path) in &candidates {
            if std::path::Path::new(path).exists() {
                return Some(format!("{}:{}", runtime, path));
            }
        }
    }

    None
}

pub(super) fn js_runtime_args() -> Vec<String> {
    let cached = {
        if let Ok(cache) = JS_RUNTIME_CACHE.read() {
            cache.clone()
        } else {
            None
        }
    };

    let runtime = match cached {
        Some(val) => val,
        None => {
            let val = detect_js_runtime();
            if let Ok(mut cache) = JS_RUNTIME_CACHE.write() {
                *cache = Some(val.clone());
            }
            val
        }
    };

    match runtime {
        Some(rt) => vec!["--js-runtimes".to_string(), rt],
        None => vec![],
    }
}

pub(super) fn is_youtube_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("youtube.com") || lower.contains("youtu.be")
}

pub(super) fn extract_error_message(stderr: &str) -> String {
    let error_line = stderr
        .lines()
        .find(|l| l.to_uppercase().contains("ERROR:"))
        .map(|l| l.trim().to_string());

    if let Some(msg) = error_line {
        return msg;
    }

    let warning_line = stderr
        .lines()
        .find(|l| l.to_uppercase().contains("WARNING:"))
        .map(|l| l.trim().to_string());

    if let Some(msg) = warning_line {
        return msg;
    }

    stderr.trim().to_string()
}

pub(super) fn default_user_agent_args(can_impersonate: bool) -> Vec<String> {
    if can_impersonate {
        Vec::new()
    } else {
        vec!["--user-agent".to_string(), CHROME_UA.to_string()]
    }
}

pub(super) fn strip_default_user_agent(args: &mut Vec<String>) {
    crate::core::impersonation::strip_default_user_agent(args, CHROME_UA);
}

pub(super) fn ytdlp_can_impersonate(ytdlp: &Path) -> bool {
    if let Some(cached) = IMPERSONATE_TARGETS.get() {
        return cached.is_some();
    }
    if is_ytdlp_zipapp(ytdlp) {
        return ZIPAPP_IMPERSONATES.get().copied().unwrap_or(false);
    }
    is_standalone_ytdlp_build(ytdlp)
}

pub(super) fn is_standalone_ytdlp_build(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 2];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && &head != b"#!"
}

#[cfg(test)]
mod cached_path_tests {
    use super::live_cached_path;

    #[test]
    fn a_remembered_miss_is_looked_up_again() {
        assert_eq!(live_cached_path(&Some(None)), None);
        assert_eq!(live_cached_path(&None), None);
    }

    #[test]
    fn a_remembered_path_is_used_only_while_the_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("yt-dlp");
        assert_eq!(live_cached_path(&Some(Some(bin.clone()))), None);
        std::fs::write(&bin, b"x").unwrap();
        assert_eq!(live_cached_path(&Some(Some(bin.clone()))), Some(bin));
    }
}
