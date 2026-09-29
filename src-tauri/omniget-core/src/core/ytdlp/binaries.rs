//! Finding, installing and updating yt-dlp and what it runs with: Python for
//! the zipapp, ffmpeg, the JS runtime, cookie files and the default user agent.

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

    // Managed builds first — they bundle yt-dlp-ejs (required for the
    // YouTube nsig challenge) and curl_cffi. System-installed yt-dlp (dnf,
    // apt, brew) often lacks both. Order and reasons: impersonation::pick_ytdlp.
    let onedir = managed_ytdlp_onedir_exe();
    if onedir.is_none() {
        spawn_onedir_install_if_missing();
    }
    let zipapp = zipapp_with_python().await;
    if zipapp.is_none() && !cfg!(target_os = "macos") {
        // macOS usa o onedir; o pyz no Python do brew vem sem curl_cffi.
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

    // Fall back to system PATH. Resolve to an absolute path so the cache
    // check (`path.exists()`) works — a bare "yt-dlp" would always fail.
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

/// Resolve a bare binary name to its absolute path via `where` (Windows)
/// or `which` (Unix). Returns the original name as fallback.
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

pub async fn find_ytdlp_cached() -> Option<PathBuf> {
    let _timer_start = std::time::Instant::now();
    if let Ok(cache) = YTDLP_PATH_CACHE.read() {
        if let Some(ref cached) = *cache {
            if let Some(ref path) = cached {
                if path.exists() {
                    tracing::debug!(
                        "[perf] find_ytdlp_cached (hit): {:?}",
                        _timer_start.elapsed()
                    );
                    return cached.clone();
                }
                tracing::warn!("[ytdlp] cached path no longer exists: {}", path.display());
            } else {
                return None;
            }
        }
    }
    let result = find_ytdlp().await;
    if let Ok(mut cache) = YTDLP_PATH_CACHE.write() {
        *cache = Some(result.clone());
    }
    tracing::debug!(
        "[perf] find_ytdlp_cached (miss): {:?}",
        _timer_start.elapsed()
    );
    result
}

/// Where the managed yt-dlp lives. Pure path resolution — it does not check
/// for the file, download it, or start anything. Callers that only need to
/// locate the binary must use this rather than [`ensure_ytdlp`], which spawns
/// a freshness check and a JS-runtime check on every call (#281).
pub fn managed_ytdlp_path() -> Option<PathBuf> {
    // Issue #222: se o usuario apontou um binario proprio, ele ganha do
    // gerenciado. Referenciado no lugar, entao um `apt upgrade` no yt-dlp dele
    // vale imediatamente aqui, sem copia para envelhecer.
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

/// Onde fica o zipapp gerido (`yt-dlp.pyz`). `None` quando o usuário apontou
/// um binário próprio: nesse caso o binário dele manda.
pub fn managed_ytdlp_zipapp_path() -> Option<PathBuf> {
    if crate::core::binary_overrides::get("yt-dlp").is_some() {
        return None;
    }
    let data = crate::core::paths::app_data_dir()?;
    Some(data.join("bin").join("yt-dlp.pyz"))
}

/// Diretorio do yt-dlp onedir gerido (`bin/yt-dlp_onedir/`). `None` fora do
/// macOS ou quando o usuario apontou um binario proprio.
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

/// Executavel do onedir gerido, so quando a extracao esta completa. Os
/// chamadores que passam o yt-dlp a um processo confinado (worker MCP) precisam
/// liberar leitura do diretorio inteiro: `_internal/` e carregado sob demanda.
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

/// Versão mínima de Python que o yt-dlp aceita.
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

/// Um Python ≥ 3.10 da máquina, se houver, para rodar o zipapp do yt-dlp.
///
/// O onefile do PyInstaller extrai 72 MB e 157 binários por lançamento, e o
/// Gatekeeper reavalia cada um: 12 s por `--version` sozinho, 30 a 90 s com a
/// fila cheia (B1). O zipapp no Python do sistema leva 0,4 s. Só vale com
/// 3.10+, que é o mínimo do yt-dlp; o `/usr/bin/python3` 3.9 do macOS não
/// serve. Consultado uma vez por processo.
pub async fn find_python() -> Option<PathBuf> {
    if let Some(cached) = PYTHON_CACHE.get() {
        return cached.clone();
    }
    tokio::task::spawn_blocking(python_cached)
        .await
        .ok()
        .flatten()
}

/// Versão síncrona (bloqueia só na primeira chamada, ~30 ms por candidato).
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

/// Comando para lançar este yt-dlp: o zipapp vai via Python, o binário direto.
// Appended after all regular arguments so engine defaults cannot override the
// external intent's single-attempt policy. Existing UI invocations are unchanged.
// Transport retries stay inside the one attempt: they resume the same file
// (a dropped connection read "6270964 bytes read, 2639932 more expected" and
// failed a Bilibili job, benchmark 26/09), bounded with exponential sleep.
// Extractor retries stay at 0: metadata requests are what trip rate limits,
// and a repeated attempt is the intent's decision, not the engine's.
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

/// Removes `--http-chunk-size <value>` from an argument list.
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

/// Baixa o zipapp em segundo plano (uma vez por sessão) quando há Python mas
/// o `.pyz` ainda não existe. Quem chamou continua com o onefile agora; o
/// próximo lançamento já pega o zipapp.
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

/// Zipapp gerido que existe e tem Python para rodar.
pub(super) async fn zipapp_with_python() -> Option<PathBuf> {
    let zip = managed_ytdlp_zipapp_path()?;
    if !zip.exists() {
        return None;
    }
    find_python().await.map(|_| zip)
}

/// Zipapp gerido pronto para uso: existe, há Python e o Python tem curl_cffi.
/// Sem impersonate o pyz não substitui o binário (Bilibili 412, Vimeo 401).
pub(super) async fn usable_zipapp() -> Option<PathBuf> {
    let zip = zipapp_with_python().await?;
    zipapp_impersonates(&zip).await.then_some(zip)
}

/// O zipapp tem alvos de `--impersonate` no Python que vai rodá-lo? Consulta
/// `--list-impersonate-targets` uma vez por processo (0,6 s); depende do
/// Python, não da versão do pyz.
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

/// Instala o onedir em segundo plano (uma vez por sessão). Quem chamou segue
/// com o binário atual; o próximo `find_ytdlp` já pega o onedir.
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

/// Primeira execução do onedir: o macOS varre `_internal/` (~20-33 s). Feita
/// no diretório em preparo, antes da troca, para nenhum download pagar isso.
pub(super) fn warm_ytdlp_onedir(exe: &Path) -> anyhow::Result<()> {
    let out = crate::core::process::std_command(exe)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| anyhow!("yt-dlp onedir nao executa: {e}"))?;
    if !out.status.success() || parse_ytdlp_version(&String::from_utf8_lossy(&out.stdout)).is_none()
    {
        return Err(anyhow!("yt-dlp onedir: --version falhou"));
    }
    Ok(())
}

pub async fn ensure_ytdlp() -> anyhow::Result<PathBuf> {
    if crate::core::dependencies::worker_mode() {
        return crate::core::dependencies::worker_tool("yt-dlp")
            .ok_or_else(|| anyhow!("WORKER_DEPENDENCY_UNAVAILABLE"));
    }
    let _timer_start = std::time::Instant::now();

    // macOS: onedir primeiro (0,4 s por processo contra 13-25 s do onefile).
    // Sem nenhum binário gerido, instala agora; havendo um, instala em segundo
    // plano e o atual segue valendo até lá.
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

    // Zipapp: sem bootstrap de 72 MB por processo. Se não há Python ou o
    // download falha, o onefile abaixo continua valendo. No macOS o onedir
    // ocupa esse papel (o Python do brew vem sem curl_cffi).
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

    // Always ensure the managed binary exists — it bundles yt-dlp-ejs and
    // works reliably with --js-runtimes and --ffmpeg-location.
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

/// O asset `yt-dlp` da release é o zipapp universal (Python ≥ 3.10).
pub(super) const YTDLP_ZIPAPP_ASSET: &str = "yt-dlp";

pub(super) async fn download_ytdlp_zipapp() -> anyhow::Result<PathBuf> {
    let target =
        managed_ytdlp_zipapp_path().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    download_ytdlp_asset(YTDLP_ZIPAPP_ASSET, target).await
}

/// Baixa o onedir oficial (`yt-dlp_macos.zip`), confere o SHA2-256SUMS da
/// release como os outros assets, extrai, aquece e troca.
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

/// Bytes de um asset da release do canal, conferidos contra o SHA2-256SUMS.
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
        .map_err(|e| anyhow!("yt-dlp: verificacao de integridade impossivel — {}", e))?;
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
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let response = client.get(&download_url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Failed to download yt-dlp: HTTP {}",
            response.status()
        ));
    }

    let bytes = response.bytes().await?;

    // Verify against the release's published checksums. Fail closed on a
    // mismatch; fail open only when the sums file itself can't be fetched, so
    // a transient GitHub hiccup doesn't block downloads entirely.
    // Fail-closed. O yt-dlp publica `SHA2-256SUMS` em toda release; não
    // conseguir buscá-lo é indistinguível de alguém suprimindo a verificação,
    // então o binário é descartado em vez de instalado sem conferência.
    let expected = integrity::expected_from_sums_url(&client, &sums_url, asset)
        .await
        .map_err(|e| anyhow!("yt-dlp: verificacao de integridade impossivel — {}", e))?;
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

    crate::core::dependencies::replace_managed_binary(&temp, &target)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&target, perms)?;
    }

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

/// Versão publicada como `releases/latest` do canal, lida do redirect
/// (`…/releases/tag/2026.08.19`). `None` quando a rede falha: sem informação
/// a regra é não mexer no binário.
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

/// Decide se vale baixar o binário do canal. Só quando a release publicada é
/// **mais nova** que a local: a regra antiga (mtime > 2 dias) rebaixava um
/// nightly 2026.08.30 para o stable 2026.08.19 e ainda trocava o arquivo com
/// downloads em andamento — o PyInstaller lê o próprio executável durante o
/// bootstrap, e um swap nesse instante vira `Traceback … pyiboot01_bootstrap`.
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
    // Nunca trocar o binário com yt-dlp rodando: o próximo lançamento faz a
    // checagem de novo.
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
                    // Reinicia a janela de 2 dias sem baixar 37 MB à toa.
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

/// Returns a disposable copy of the extension cookie file for yt-dlp.
///
/// yt-dlp rewrites `--cookies` files after every run, which corrupts the
/// original cookies written by the Chrome extension. We copy the source
/// file to a sibling temp file so yt-dlp mutates the copy, not the original.
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

/// Detect a JavaScript runtime for yt-dlp's nsig challenge solver.
/// yt-dlp standalone binaries cannot discover runtimes from PATH on their
/// own, so we locate the binary and pass it via `--js-runtimes runtime:path`.
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

    // Try system PATH via `where` (Windows) or `which` (Unix).
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

    // Check managed bin dir (Deno auto-downloaded alongside yt-dlp).
    if let Some(bin_dir) = crate::core::paths::app_data_dir().map(|d| d.join("bin")) {
        for &(runtime, bin) in runtimes {
            let managed = bin_dir.join(bin);
            if managed.exists() {
                return Some(format!("{}:{}", runtime, managed.display()));
            }
        }
    }

    // Fallback: well-known install locations on Windows.
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

/// Extracts the most meaningful error line from yt-dlp stderr output.
/// Prefers lines starting with "ERROR:", falls back to "WARNING:", then raw trimmed output.
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

/// `--user-agent` padrão do app, só para build sem impersonate. Com curl_cffi
/// o yt-dlp casa UA e TLS sozinho; o nosso Chrome/131 de Windows junto do TLS
/// do Chrome 146/macOS é uma incoerência que Instagram/TikTok/Reddit veem.
pub(super) fn default_user_agent_args(can_impersonate: bool) -> Vec<String> {
    if can_impersonate {
        Vec::new()
    } else {
        vec!["--user-agent".to_string(), CHROME_UA.to_string()]
    }
}

/// Com `--impersonate` no comando, o UA padrão sai (o da extensão/config fica).
pub(super) fn strip_default_user_agent(args: &mut Vec<String>) {
    crate::core::impersonation::strip_default_user_agent(args, CHROME_UA);
}

/// Este yt-dlp imita navegador (tem curl_cffi)? Sem processo novo: usa a
/// lista já consultada, a sondagem do zipapp, ou reconhece o build
/// standalone oficial (PyInstaller, traz curl_cffi). yt-dlp de pacote (brew,
/// pip) é script Python (`#!`) e fica com o UA padrão.
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
