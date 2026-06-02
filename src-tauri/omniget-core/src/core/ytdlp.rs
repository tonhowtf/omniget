use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::anyhow;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::core::log_hook;
use crate::models::media::{DownloadResult, FormatInfo};
use crate::models::progress::ProgressUpdate;

type ExtCookiePathFn = Box<dyn Fn() -> PathBuf + Send + Sync>;
type GlobalCookieFileFn = Box<dyn Fn() -> Option<String> + Send + Sync>;
type CookiesFromBrowserFn = Box<dyn Fn() -> String + Send + Sync>;
type ManualCookieHeaderFn = Box<dyn Fn() -> String + Send + Sync>;
type ExtRefererFn = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;
type IncludeAutoSubsFn = Box<dyn Fn() -> bool + Send + Sync>;
type CaptionLocaleFn = Box<dyn Fn() -> String + Send + Sync>;
type KeepVttFn = Box<dyn Fn() -> bool + Send + Sync>;
type TranslateMetadataFn = Box<dyn Fn() -> Option<String> + Send + Sync>;
type SponsorBlockFn = Box<dyn Fn() -> bool + Send + Sync>;
type SplitChaptersFn = Box<dyn Fn() -> bool + Send + Sync>;
type EmbedMetadataFn = Box<dyn Fn() -> bool + Send + Sync>;
type EmbedThumbnailFn = Box<dyn Fn() -> bool + Send + Sync>;
type SpeedLimitFn = Box<dyn Fn() -> Option<String> + Send + Sync>;
type LiveFromStartFn = Box<dyn Fn() -> bool + Send + Sync>;
type ConcurrentFragmentsFn = Box<dyn Fn() -> u32 + Send + Sync>;
type UserAgentFn = Box<dyn Fn() -> Option<String> + Send + Sync>;
type SponsorBlockModeFn = Box<dyn Fn() -> String + Send + Sync>;
type SponsorBlockCategoriesFn = Box<dyn Fn() -> Vec<String> + Send + Sync>;
type PerDomainCookieFn = Box<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;
type ManagedCookiesOnlyFn = Box<dyn Fn() -> bool + Send + Sync>;

static EXT_COOKIE_PATH_FN: OnceLock<ExtCookiePathFn> = OnceLock::new();
static GLOBAL_COOKIE_FILE_FN: OnceLock<GlobalCookieFileFn> = OnceLock::new();
static COOKIES_FROM_BROWSER_FN: OnceLock<CookiesFromBrowserFn> = OnceLock::new();
static MANUAL_COOKIE_HEADER_FN: OnceLock<ManualCookieHeaderFn> = OnceLock::new();
static EXT_REFERER_FN: OnceLock<ExtRefererFn> = OnceLock::new();
static INCLUDE_AUTO_SUBS_FN: OnceLock<IncludeAutoSubsFn> = OnceLock::new();
static CAPTION_LOCALE_FN: OnceLock<CaptionLocaleFn> = OnceLock::new();
static KEEP_VTT_FN: OnceLock<KeepVttFn> = OnceLock::new();
static PER_DOMAIN_COOKIE_FN: OnceLock<PerDomainCookieFn> = OnceLock::new();
static MANAGED_COOKIES_ONLY_FN: OnceLock<ManagedCookiesOnlyFn> = OnceLock::new();
static TRANSLATE_METADATA_FN: OnceLock<TranslateMetadataFn> = OnceLock::new();
static SPONSORBLOCK_FN: OnceLock<SponsorBlockFn> = OnceLock::new();
static SPLIT_CHAPTERS_FN: OnceLock<SplitChaptersFn> = OnceLock::new();
static EMBED_METADATA_FN: OnceLock<EmbedMetadataFn> = OnceLock::new();
static EMBED_THUMBNAIL_FN: OnceLock<EmbedThumbnailFn> = OnceLock::new();
static SPEED_LIMIT_FN: OnceLock<SpeedLimitFn> = OnceLock::new();
static LIVE_FROM_START_FN: OnceLock<LiveFromStartFn> = OnceLock::new();
static CONCURRENT_FRAGMENTS_FN: OnceLock<ConcurrentFragmentsFn> = OnceLock::new();
static USER_AGENT_FN: OnceLock<UserAgentFn> = OnceLock::new();
static SPONSORBLOCK_MODE_FN: OnceLock<SponsorBlockModeFn> = OnceLock::new();
static SPONSORBLOCK_CATEGORIES_FN: OnceLock<SponsorBlockCategoriesFn> = OnceLock::new();

pub fn set_ext_cookie_path_fn(f: impl Fn() -> PathBuf + Send + Sync + 'static) {
    let _ = EXT_COOKIE_PATH_FN.set(Box::new(f));
}

pub fn set_per_domain_cookie_fn(f: impl Fn(&str) -> Option<PathBuf> + Send + Sync + 'static) {
    let _ = PER_DOMAIN_COOKIE_FN.set(Box::new(f));
}

pub fn set_managed_cookies_only_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = MANAGED_COOKIES_ONLY_FN.set(Box::new(f));
}

fn managed_cookies_only() -> bool {
    MANAGED_COOKIES_ONLY_FN.get().map(|f| f()).unwrap_or(true)
}

pub fn set_global_cookie_file_fn(f: impl Fn() -> Option<String> + Send + Sync + 'static) {
    let _ = GLOBAL_COOKIE_FILE_FN.set(Box::new(f));
}

pub fn set_cookies_from_browser_fn(f: impl Fn() -> String + Send + Sync + 'static) {
    let _ = COOKIES_FROM_BROWSER_FN.set(Box::new(f));
}

pub fn set_manual_cookie_header_fn(f: impl Fn() -> String + Send + Sync + 'static) {
    let _ = MANUAL_COOKIE_HEADER_FN.set(Box::new(f));
}

pub fn set_ext_referer_fn(f: impl Fn(&str) -> Option<String> + Send + Sync + 'static) {
    let _ = EXT_REFERER_FN.set(Box::new(f));
}

pub fn set_include_auto_subs_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = INCLUDE_AUTO_SUBS_FN.set(Box::new(f));
}

fn include_auto_subs_setting() -> bool {
    INCLUDE_AUTO_SUBS_FN.get().map(|f| f()).unwrap_or(false)
}

pub fn set_caption_locale_fn(f: impl Fn() -> String + Send + Sync + 'static) {
    let _ = CAPTION_LOCALE_FN.set(Box::new(f));
}

fn caption_locale_setting() -> String {
    let lang = CAPTION_LOCALE_FN
        .get()
        .map(|f| f())
        .unwrap_or_else(|| "en".to_string());
    let lang = lang.trim();
    if lang.is_empty() {
        "en".to_string()
    } else {
        lang.to_string()
    }
}

fn requested_caption_locales() -> Vec<String> {
    caption_locale_setting()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub fn set_keep_vtt_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = KEEP_VTT_FN.set(Box::new(f));
}

fn keep_vtt_setting() -> bool {
    KEEP_VTT_FN.get().map(|f| f()).unwrap_or(false)
}

pub fn set_translate_metadata_fn(f: impl Fn() -> Option<String> + Send + Sync + 'static) {
    let _ = TRANSLATE_METADATA_FN.set(Box::new(f));
}

fn translate_metadata_lang() -> Option<String> {
    TRANSLATE_METADATA_FN.get().and_then(|f| f())
}

static ACTIVE_PROCESS_PIDS: OnceLock<std::sync::Mutex<HashMap<u64, u32>>> = OnceLock::new();

fn active_process_pids() -> &'static std::sync::Mutex<HashMap<u64, u32>> {
    ACTIVE_PROCESS_PIDS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn register_download_process(download_id: u64, pid: u32) {
    if let Ok(mut pids) = active_process_pids().lock() {
        pids.insert(download_id, pid);
    }
}

fn unregister_download_process(download_id: u64) {
    if let Ok(mut pids) = active_process_pids().lock() {
        pids.remove(&download_id);
    }
}

#[cfg(unix)]
fn signal_download_process(download_id: u64, signal: &str) -> bool {
    let pid = active_process_pids()
        .lock()
        .ok()
        .and_then(|pids| pids.get(&download_id).copied());
    let Some(pid) = pid else {
        return false;
    };
    std::process::Command::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn signal_download_process(_download_id: u64, _signal: &str) -> bool {
    false
}

pub fn pause_download_process(download_id: u64) -> bool {
    signal_download_process(download_id, "-STOP")
}

pub fn resume_download_process(download_id: u64) -> bool {
    signal_download_process(download_id, "-CONT")
}

fn normalize_youtube_lang(lang: &str) -> String {
    match lang.trim() {
        "zh" | "zh-CN" | "zh-cn" | "zh-Hans" | "zh-hans" => "zh-CN".to_string(),
        "zh-TW" | "zh-tw" | "zh-Hant" | "zh-hant" => "zh-TW".to_string(),
        "zh-HK" | "zh-hk" => "zh-HK".to_string(),
        other => other.to_string(),
    }
}

pub fn set_sponsorblock_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = SPONSORBLOCK_FN.set(Box::new(f));
}

fn sponsorblock_enabled() -> bool {
    SPONSORBLOCK_FN.get().map(|f| f()).unwrap_or(false)
}

pub fn set_sponsorblock_mode_fn(f: impl Fn() -> String + Send + Sync + 'static) {
    let _ = SPONSORBLOCK_MODE_FN.set(Box::new(f));
}

fn sponsorblock_mode() -> String {
    SPONSORBLOCK_MODE_FN
        .get()
        .map(|f| f())
        .unwrap_or_else(|| "remove".to_string())
}

pub fn set_sponsorblock_categories_fn(f: impl Fn() -> Vec<String> + Send + Sync + 'static) {
    let _ = SPONSORBLOCK_CATEGORIES_FN.set(Box::new(f));
}

fn sponsorblock_categories() -> Vec<String> {
    SPONSORBLOCK_CATEGORIES_FN
        .get()
        .map(|f| f())
        .unwrap_or_default()
}

pub fn set_split_chapters_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = SPLIT_CHAPTERS_FN.set(Box::new(f));
}

fn split_chapters_enabled() -> bool {
    SPLIT_CHAPTERS_FN.get().map(|f| f()).unwrap_or(false)
}

pub fn set_embed_metadata_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = EMBED_METADATA_FN.set(Box::new(f));
}

fn embed_metadata_enabled() -> bool {
    EMBED_METADATA_FN.get().map(|f| f()).unwrap_or(true)
}

pub fn set_embed_thumbnail_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = EMBED_THUMBNAIL_FN.set(Box::new(f));
}

fn embed_thumbnail_enabled() -> bool {
    EMBED_THUMBNAIL_FN.get().map(|f| f()).unwrap_or(true)
}

pub fn set_speed_limit_fn(f: impl Fn() -> Option<String> + Send + Sync + 'static) {
    let _ = SPEED_LIMIT_FN.set(Box::new(f));
}

fn speed_limit_value() -> Option<String> {
    SPEED_LIMIT_FN.get().and_then(|f| f())
}

pub fn set_live_from_start_fn(f: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = LIVE_FROM_START_FN.set(Box::new(f));
}

fn live_from_start_enabled() -> bool {
    LIVE_FROM_START_FN.get().map(|f| f()).unwrap_or(false)
}

pub fn set_concurrent_fragments_fn(f: impl Fn() -> u32 + Send + Sync + 'static) {
    let _ = CONCURRENT_FRAGMENTS_FN.set(Box::new(f));
}

fn concurrent_fragments_value() -> u32 {
    CONCURRENT_FRAGMENTS_FN.get().map(|f| f()).unwrap_or(1)
}

pub fn set_user_agent_fn(f: impl Fn() -> Option<String> + Send + Sync + 'static) {
    let _ = USER_AGENT_FN.set(Box::new(f));
}

fn user_agent_setting() -> Option<String> {
    USER_AGENT_FN.get().and_then(|f| f())
}

static EXT_UA_MAP: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn ext_ua_map() -> &'static Mutex<HashMap<String, String>> {
    EXT_UA_MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register_ext_user_agent(url: String, ua: String) {
    if let Ok(mut map) = ext_ua_map().lock() {
        map.insert(url, ua);
    }
}

pub fn clear_ext_user_agent(url: &str) {
    if let Ok(mut map) = ext_ua_map().lock() {
        map.remove(url);
    }
}

fn ext_user_agent_for_url(url: &str) -> Option<String> {
    ext_ua_map().lock().ok().and_then(|m| m.get(url).cloned())
}

static ETA_BY_DOWNLOAD: OnceLock<Mutex<HashMap<u64, u64>>> = OnceLock::new();

fn eta_map() -> &'static Mutex<HashMap<u64, u64>> {
    ETA_BY_DOWNLOAD.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn record_eta(download_id: u64, eta_seconds: u64) {
    if let Ok(mut m) = eta_map().lock() {
        m.insert(download_id, eta_seconds);
    }
}

pub fn get_eta(download_id: u64) -> Option<u64> {
    eta_map()
        .lock()
        .ok()
        .and_then(|m| m.get(&download_id).copied())
}

pub fn clear_eta(download_id: u64) {
    if let Ok(mut m) = eta_map().lock() {
        m.remove(&download_id);
    }
}

static EXT_HEADERS_MAP: OnceLock<Mutex<HashMap<String, HashMap<String, String>>>> = OnceLock::new();

fn ext_headers_map() -> &'static Mutex<HashMap<String, HashMap<String, String>>> {
    EXT_HEADERS_MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register_ext_headers(url: String, headers: HashMap<String, String>) {
    if let Ok(mut map) = ext_headers_map().lock() {
        map.insert(url, headers);
    }
}

pub fn clear_ext_headers(url: &str) {
    if let Ok(mut map) = ext_headers_map().lock() {
        map.remove(url);
    }
}

fn ext_headers_for_url(url: &str) -> Option<HashMap<String, String>> {
    ext_headers_map()
        .lock()
        .ok()
        .and_then(|m| m.get(url).cloned())
}

fn ext_referer_for_url(url: &str) -> Option<String> {
    EXT_REFERER_FN.get().and_then(|f| f(url))
}

fn cookies_from_browser_setting() -> String {
    COOKIES_FROM_BROWSER_FN
        .get()
        .map(|f| f())
        .unwrap_or_default()
}

fn manual_cookie_header_setting() -> Option<String> {
    let raw = MANUAL_COOKIE_HEADER_FN
        .get()
        .map(|f| f())
        .unwrap_or_default();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parsed = crate::core::cookie_parser::parse_cookie_input(trimmed, "");
    if !parsed.cookie_string.trim().is_empty() {
        Some(parsed.cookie_string)
    } else {
        Some(trimmed.to_string())
    }
}

pub fn ext_cookie_path() -> PathBuf {
    EXT_COOKIE_PATH_FN
        .get()
        .map(|f| f())
        .unwrap_or_else(|| PathBuf::from(""))
}

pub fn ext_cookie_path_if_fresh() -> Option<PathBuf> {
    let source = ext_cookie_path();
    if !source.exists() {
        return None;
    }
    let metadata = std::fs::metadata(&source).ok()?;
    let modified = metadata.modified().ok()?;
    if modified.elapsed().unwrap_or_default() >= std::time::Duration::from_secs(604800) {
        return None;
    }
    Some(source)
}

fn global_cookie_file() -> Option<String> {
    GLOBAL_COOKIE_FILE_FN.get().and_then(|f| f())
}

static YTDLP_UPDATING: AtomicBool = AtomicBool::new(false);
static YTDLP_UPDATE_CHECKED: AtomicBool = AtomicBool::new(false);
static YTDLP_PATH_CACHE: std::sync::RwLock<Option<Option<PathBuf>>> = std::sync::RwLock::new(None);
static FFMPEG_LOCATION_CACHE: std::sync::RwLock<Option<Option<String>>> =
    std::sync::RwLock::new(None);
static JS_RUNTIME_CACHE: std::sync::RwLock<Option<Option<String>>> = std::sync::RwLock::new(None);
static RATE_LIMIT_429_COUNT: AtomicU64 = AtomicU64::new(0);
static RATE_LIMIT_429_LAST_TS: AtomicU64 = AtomicU64::new(0);
static COOKIE_ERROR_FLAG: AtomicBool = AtomicBool::new(false);

pub fn has_cookie_error() -> bool {
    COOKIE_ERROR_FLAG.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn clear_cookie_error() {
    COOKIE_ERROR_FLAG.store(false, std::sync::atomic::Ordering::Relaxed);
}

fn rate_limit_429_count() -> u64 {
    let last = RATE_LIMIT_429_LAST_TS.load(Ordering::Relaxed);
    if last == 0 {
        return 0;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now.saturating_sub(last) > 1800 {
        RATE_LIMIT_429_COUNT.store(0, Ordering::Relaxed);
        RATE_LIMIT_429_LAST_TS.store(0, Ordering::Relaxed);
        return 0;
    }
    RATE_LIMIT_429_COUNT.load(Ordering::Relaxed)
}

fn rate_limit_429_increment() {
    RATE_LIMIT_429_COUNT.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    RATE_LIMIT_429_LAST_TS.store(now, Ordering::Relaxed);
}

pub fn reset_ytdlp_cache() {
    if let Ok(mut cache) = YTDLP_PATH_CACHE.write() {
        *cache = None;
    }
}

pub fn reset_ffmpeg_location_cache() {
    if let Ok(mut cache) = FFMPEG_LOCATION_CACHE.write() {
        *cache = None;
    }
}

pub fn reset_js_runtime_cache() {
    if let Ok(mut cache) = JS_RUNTIME_CACHE.write() {
        *cache = None;
    }
}

pub async fn check_ytdlp_update(ytdlp: &Path) -> anyhow::Result<bool> {
    if YTDLP_UPDATE_CHECKED.swap(true, Ordering::Relaxed) {
        return Ok(false);
    }

    let ytdlp = ytdlp.to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        crate::core::process::std_command(&ytdlp)
            .args(["--update-to", "nightly"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    })
    .await??;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);

    if combined.contains("Updated yt-dlp") || combined.contains("Updating to") {
        tracing::info!("[ytdlp] updated: {}", combined.trim());
        reset_ytdlp_cache();
        Ok(true)
    } else {
        Ok(false)
    }
}

fn proxy_args() -> Vec<String> {
    match crate::core::http_client::proxy_url() {
        Some(url) => vec!["--proxy".to_string(), url],
        None => Vec::new(),
    }
}

fn has_explicit_cookie_header(args: &[String]) -> bool {
    args.windows(2).any(|pair| {
        pair[0] == "--add-headers" && pair[1].to_ascii_lowercase().starts_with("cookie:")
    })
}

fn append_cookie_header(args: &mut Vec<String>, cookie_header: &str) {
    args.push("--add-headers".to_string());
    args.push(format!("Cookie:{}", cookie_header));
}

enum MetadataCookieSource {
    PerDomain(PathBuf),
    ManualHeader(String),
    CookieFile(PathBuf),
    Browser(String),
    None,
    ExplicitHeader,
}

fn metadata_cookie_source(url: &str, extra_flags: &[String]) -> MetadataCookieSource {
    if has_explicit_cookie_header(extra_flags) {
        return MetadataCookieSource::ExplicitHeader;
    }

    if let Some(path) = per_domain_cookie_file(url) {
        return MetadataCookieSource::PerDomain(path);
    }

    if let Some(header) = manual_cookie_header_setting() {
        return MetadataCookieSource::ManualHeader(header);
    }

    if let Some(path) = extension_cookie_file() {
        return MetadataCookieSource::CookieFile(path);
    }

    if let Some(path) = global_cookie_file().map(PathBuf::from) {
        return MetadataCookieSource::CookieFile(path);
    }

    let browser = cookies_from_browser_setting();
    if !browser.is_empty() {
        return MetadataCookieSource::Browser(browser);
    }

    MetadataCookieSource::None
}

fn append_metadata_cookie_args(
    args: &mut Vec<String>,
    url: &str,
    extra_flags: &[String],
    context: &str,
) {
    match metadata_cookie_source(url, extra_flags) {
        MetadataCookieSource::PerDomain(path) => {
            args.push("--cookies".to_string());
            args.push(path.to_string_lossy().to_string());
            tracing::debug!("[yt-dlp] using per-domain cookies for {}", context);
        }
        MetadataCookieSource::ManualHeader(header) => {
            append_cookie_header(args, &header);
            tracing::debug!("[yt-dlp] using manual cookie header for {}", context);
        }
        MetadataCookieSource::CookieFile(path) => {
            args.push("--cookies".to_string());
            args.push(path.to_string_lossy().to_string());
        }
        MetadataCookieSource::Browser(browser) => {
            args.push("--cookies-from-browser".to_string());
            args.push(browser);
        }
        MetadataCookieSource::ExplicitHeader => {
            tracing::debug!(
                "[yt-dlp] skipping cookies-from-browser for {} because explicit Cookie header was provided",
                context
            );
        }
        MetadataCookieSource::None => {}
    }
}

struct YtRateLimiter {
    semaphore: tokio::sync::Semaphore,
    last_request_ns: AtomicU64,
}

impl YtRateLimiter {
    async fn acquire(&self) {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .unwrap_or_else(|_| panic!("semaphore closed"));
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
            tracing::debug!("[perf] find_ytdlp took {:?}", _timer_start.elapsed());
            return Some(flatpak_path);
        }
    }

    // Prefer the managed binary — it bundles yt-dlp-ejs (required for
    // YouTube nsig challenge). System-installed yt-dlp (dnf, apt) often
    // lacks this plugin, causing "Requested format is not available".
    let managed = managed_ytdlp_path()?;
    if managed.exists() {
        tracing::debug!("[perf] find_ytdlp took {:?}", _timer_start.elapsed());
        return Some(managed);
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

fn managed_ytdlp_path() -> Option<PathBuf> {
    let data = crate::core::paths::app_data_dir()?;
    let bin_name = if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };
    Some(data.join("bin").join(bin_name))
}

pub async fn ensure_ytdlp() -> anyhow::Result<PathBuf> {
    let _timer_start = std::time::Instant::now();

    // Always ensure the managed binary exists — it bundles yt-dlp-ejs and
    // works reliably with --js-runtimes and --ffmpeg-location.
    if !crate::core::dependencies::is_flatpak() {
        let managed = managed_ytdlp_path();
        if managed.as_ref().map_or(true, |p| !p.exists()) {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YtdlpChannel {
    Stable,
    Nightly,
}

fn ytdlp_channel() -> YtdlpChannel {
    match std::env::var("OMNIGET_YTDLP_CHANNEL") {
        Ok(v) if v.trim().eq_ignore_ascii_case("nightly") => YtdlpChannel::Nightly,
        _ => YtdlpChannel::Stable,
    }
}

fn ytdlp_asset_name() -> &'static str {
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

fn ytdlp_release_base(channel: YtdlpChannel) -> &'static str {
    match channel {
        YtdlpChannel::Stable => "https://github.com/yt-dlp/yt-dlp/releases/latest/download",
        YtdlpChannel::Nightly => {
            "https://github.com/yt-dlp/yt-dlp-nightly-builds/releases/latest/download"
        }
    }
}

/// Finds the expected sha256 (lowercased hex) for `asset` in a yt-dlp
/// `SHA2-256SUMS` file. Lines are `"<64-hex>  <filename>"`.
fn parse_sha256sums(text: &str, asset: &str) -> Option<String> {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?;
        if name == asset && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(hash.to_lowercase());
        }
    }
    None
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

async fn download_ytdlp_binary() -> anyhow::Result<PathBuf> {
    let target =
        managed_ytdlp_path().ok_or_else(|| anyhow!("Could not determine data directory"))?;

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let channel = ytdlp_channel();
    let asset = ytdlp_asset_name();
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
    match client.get(&sums_url).send().await {
        Ok(r) if r.status().is_success() => {
            let sums = r.text().await.unwrap_or_default();
            match parse_sha256sums(&sums, asset) {
                Some(expected) => {
                    let actual = sha256_hex(&bytes);
                    if actual != expected {
                        return Err(anyhow!(
                            "yt-dlp checksum mismatch (expected {}, got {}) — refusing to install",
                            expected,
                            actual
                        ));
                    }
                    tracing::info!("[ytdlp] sha256 verified ({:?} channel)", channel);
                }
                None => tracing::warn!(
                    "[ytdlp] {} not listed in SHA2-256SUMS — skipping verification",
                    asset
                ),
            }
        }
        _ => tracing::warn!("[ytdlp] could not fetch SHA2-256SUMS — skipping verification"),
    }

    let target_clone = target.clone();
    tokio::task::spawn_blocking(move || std::fs::write(&target_clone, &bytes))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {}", e))??;

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

async fn check_ytdlp_freshness(path: &Path) {
    if let Some(managed) = managed_ytdlp_path() {
        if path != managed.as_path() {
            return;
        }
    } else {
        return;
    }

    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(modified) = meta.modified() {
            if let Ok(age) = modified.elapsed() {
                if age > std::time::Duration::from_secs(2 * 24 * 60 * 60) {
                    if YTDLP_UPDATING
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                        .is_err()
                    {
                        return;
                    }
                    tracing::info!("yt-dlp is older than 2 days, updating in background");
                    std::thread::Builder::new()
                        .name("ytdlp-update".into())
                        .spawn(|| {
                            let rt = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                                .expect("ytdlp-update runtime");
                            rt.block_on(async {
                                match download_ytdlp_binary().await {
                                    Ok(_) => tracing::info!("yt-dlp updated successfully"),
                                    Err(e) => tracing::warn!("Failed to update yt-dlp: {}", e),
                                }
                                YTDLP_UPDATING.store(false, Ordering::SeqCst);
                            });
                        })
                        .ok();
                }
            }
        }
    }
}

async fn find_ffmpeg_location() -> Option<String> {
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

async fn find_ffmpeg_location_cached() -> Option<String> {
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

fn per_domain_cookie_file(url: &str) -> Option<std::path::PathBuf> {
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
fn extension_cookie_file() -> Option<std::path::PathBuf> {
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
fn detect_js_runtime() -> Option<String> {
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

fn js_runtime_args() -> Vec<String> {
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

fn is_youtube_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("youtube.com") || lower.contains("youtu.be")
}

/// Extracts the most meaningful error line from yt-dlp stderr output.
/// Prefers lines starting with "ERROR:", falls back to "WARNING:", then raw trimmed output.
fn extract_error_message(stderr: &str) -> String {
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

pub async fn get_video_info(
    ytdlp: &Path,
    url: &str,
    extra_flags: &[String],
) -> anyhow::Result<serde_json::Value> {
    let _timer_start = std::time::Instant::now();

    if is_youtube_url(url) {
        yt_rate_limiter().acquire().await;
    }

    let is_yt = is_youtube_url(url);
    let clients: &[Option<&str>] = if is_yt {
        &[None, Some("youtube:player_client=default,mweb")]
    } else {
        &[None]
    };

    let mut last_error = String::new();

    for (attempt, client) in clients.iter().enumerate() {
        tracing::info!(
            "[yt-dlp] info fetch attempt {}/{} for URL",
            attempt + 1,
            clients.len()
        );

        let mut args = vec![
            "--dump-single-json".to_string(),
            "--no-warnings".to_string(),
            "--no-playlist".to_string(),
            "--no-check-certificates".to_string(),
            "--encoding".to_string(),
            "utf-8".to_string(),
            "--socket-timeout".to_string(),
            "15".to_string(),
            "--retries".to_string(),
            "1".to_string(),
            "--extractor-retries".to_string(),
            "2".to_string(),
            "--retry-sleep".to_string(),
            "exp=1:30".to_string(),
            "--user-agent".to_string(),
            CHROME_UA.to_string(),
            "--skip-download".to_string(),
        ];
        args.extend(js_runtime_args());

        if let Some(extractor_args) = client {
            args.push("--extractor-args".to_string());
            args.push(extractor_args.to_string());
        }

        append_metadata_cookie_args(&mut args, url, extra_flags, "video info");

        args.extend(proxy_args());
        args.extend(extra_flags.iter().cloned());
        args.push(url.to_string());

        let child = crate::core::process::command(ytdlp)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("Failed to run yt-dlp: {}", e))?;
        tracing::debug!(
            "[perf] get_video_info: yt-dlp process spawned at {:?} (attempt {})",
            _timer_start.elapsed(),
            attempt + 1
        );

        let result =
            tokio::time::timeout(std::time::Duration::from_secs(60), child.wait_with_output())
                .await
                .map_err(|_| {
                    tracing::debug!("[perf] get_video_info took {:?}", _timer_start.elapsed());
                    anyhow!("Timeout fetching video info (60s)")
                })?
                .map_err(|e| {
                    tracing::debug!("[perf] get_video_info took {:?}", _timer_start.elapsed());
                    anyhow!("Failed to run yt-dlp: {}", e)
                })?;

        tracing::debug!(
            "[perf] get_video_info: yt-dlp process exited at {:?} (attempt {})",
            _timer_start.elapsed(),
            attempt + 1
        );

        if result.status.success() {
            let json: serde_json::Value = serde_json::from_slice(&result.stdout)
                .map_err(|e| anyhow!("yt-dlp returned invalid JSON: {}", e))?;
            tracing::debug!("[perf] get_video_info took {:?}", _timer_start.elapsed());
            return Ok(json);
        }

        let stderr = String::from_utf8_lossy(&result.stderr).to_string();
        tracing::debug!(
            "[yt-dlp info] stderr ({} bytes): {}",
            stderr.len(),
            stderr.trim()
        );
        let stderr_lower = stderr.to_lowercase();
        if stderr_lower.contains("http error 429") {
            rate_limit_429_increment();
            let sanitized_url = sanitize_log_line(url);
            tracing::warn!(
                "[yt-429] rate limit in get_video_info: url={} attempt={}/{}",
                sanitized_url,
                attempt + 1,
                clients.len()
            );
        }

        let is_retryable = is_yt
            && attempt < clients.len() - 1
            && (stderr_lower.contains("requested format")
                || stderr_lower.contains("not available")
                || stderr_lower.contains("http error 403")
                || stderr_lower.contains("nsig")
                || stderr_lower.contains("http error 429"));

        if is_retryable {
            tracing::warn!(
                "[yt-dlp] info fetch attempt {} failed, retrying with fallback player_client: {}",
                attempt + 1,
                stderr.trim().lines().last().unwrap_or("")
            );
            if stderr_lower.contains("http error 429") {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
            last_error = stderr;
            continue;
        }

        tracing::debug!("[perf] get_video_info took {:?}", _timer_start.elapsed());
        return Err(translate_ytdlp_error(&stderr));
    }

    tracing::debug!("[perf] get_video_info took {:?}", _timer_start.elapsed());
    Err(translate_ytdlp_error(&last_error))
}

async fn select_available_subtitle_lang(
    ytdlp: &Path,
    url: &str,
    extra_flags: &[String],
    include_auto: bool,
) -> anyhow::Result<Option<String>> {
    let requested = requested_caption_locales();
    let json = get_video_info(ytdlp, url, extra_flags).await?;
    let (manual, auto) = subtitle_languages_from_json(&json);
    let available: Vec<String> = if include_auto {
        manual.iter().chain(auto.iter()).cloned().collect()
    } else {
        manual.clone()
    };

    let selected = requested
        .iter()
        .find_map(|lang| matching_subtitle_lang(lang, &available));
    if selected.is_some() {
        return Ok(selected);
    }

    let requested_label = if requested.is_empty() {
        "(empty)".to_string()
    } else {
        requested.join(", ")
    };
    let manual_label = format_lang_list(&manual);
    let auto_label = format_lang_list(&auto);
    let line = if manual.is_empty() && auto.is_empty() {
        format!(
            "[subtitles] 没有可下载字幕；继续下载视频，不下载字幕。您选择的字幕语言: {}",
            requested_label
        )
    } else if manual.is_empty() && !auto.is_empty() && !include_auto {
        format!(
            "[subtitles] 没有手工字幕；自动字幕可用但未启用。继续下载视频，不下载字幕。您选择: {}; 自动字幕: {}",
            requested_label, auto_label
        )
    } else {
        format!(
            "[subtitles] 没有您选择的字幕；继续下载视频，不下载字幕。您选择: {}; 可用手工字幕: {}; 可用自动字幕: {}",
            requested_label, manual_label, auto_label
        )
    };
    tracing::warn!("{}", line);
    if let Some(dl_id) = log_hook::current_download_id() {
        log_hook::emit_log(dl_id, &line);
    }

    Ok(None)
}

fn subtitle_languages_from_json(json: &serde_json::Value) -> (Vec<String>, Vec<String>) {
    fn collect(map: Option<&serde_json::Value>) -> Vec<String> {
        let mut langs = Vec::new();
        if let Some(obj) = map.and_then(|v| v.as_object()) {
            for (lang, formats) in obj {
                if lang == "live_chat" {
                    continue;
                }
                if formats.as_array().map(|a| !a.is_empty()).unwrap_or(false) {
                    langs.push(lang.clone());
                }
            }
        }
        langs.sort_by_key(|s| s.to_ascii_lowercase());
        langs.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        langs
    }

    (
        collect(json.get("subtitles")),
        collect(json.get("automatic_captions")),
    )
}

fn matching_subtitle_lang(requested: &str, available: &[String]) -> Option<String> {
    available
        .iter()
        .find(|lang| normalize_lang_token(lang) == normalize_lang_token(requested))
        .cloned()
        .or_else(|| {
            available
                .iter()
                .find(|lang| subtitle_lang_matches(requested, lang))
                .cloned()
        })
}

fn subtitle_lang_matches(requested: &str, available: &str) -> bool {
    let requested = normalize_lang_token(requested);
    let available = normalize_lang_token(available);
    if requested.is_empty() || available.is_empty() {
        return false;
    }
    if requested == available {
        return true;
    }

    let req_base = requested.split('-').next().unwrap_or("");
    let avail_base = available.split('-').next().unwrap_or("");
    if req_base != avail_base {
        return false;
    }

    if req_base == "zh" {
        return zh_subtitle_group(&requested) == zh_subtitle_group(&available)
            || requested == "zh"
            || available == "zh";
    }

    requested == req_base || available == avail_base
}

fn normalize_lang_token(lang: &str) -> String {
    lang.trim().replace('_', "-").to_ascii_lowercase()
}

fn zh_subtitle_group(lang: &str) -> &'static str {
    match lang {
        "zh-tw" | "zh-hant" | "zh-hk" | "zh-mo" => "traditional",
        _ => "simplified",
    }
}

fn format_lang_list(langs: &[String]) -> String {
    if langs.is_empty() {
        "none".to_string()
    } else {
        langs.join(", ")
    }
}

pub async fn get_playlist_info(
    ytdlp: &Path,
    url: &str,
    extra_flags: &[String],
) -> anyhow::Result<(String, Vec<PlaylistEntry>)> {
    if is_youtube_url(url) {
        yt_rate_limiter().acquire().await;
    }

    let mut args = vec![
        "--flat-playlist".to_string(),
        "--dump-json".to_string(),
        "--no-warnings".to_string(),
        "--encoding".to_string(),
        "utf-8".to_string(),
        "--socket-timeout".to_string(),
        "30".to_string(),
        "--retries".to_string(),
        "3".to_string(),
        "--extractor-retries".to_string(),
        "3".to_string(),
        "--retry-sleep".to_string(),
        "exp=1:15".to_string(),
        "--user-agent".to_string(),
        CHROME_UA.to_string(),
    ];
    args.extend(js_runtime_args());

    if is_youtube_url(url) {
        args.push("--extractor-args".to_string());
        args.push("youtube:player_client=default".to_string());
    }

    append_metadata_cookie_args(&mut args, url, extra_flags, "playlist info");

    args.extend(proxy_args());
    args.extend(extra_flags.iter().cloned());
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
            rate_limit_429_increment();
            let sanitized_url = sanitize_log_line(url);
            let player_client = if is_youtube_url(url) {
                "default"
            } else {
                "n/a"
            };
            tracing::warn!(
                "[yt-429] rate limit in get_playlist_info: url={} player_client={} retries=3",
                sanitized_url,
                player_client
            );
        }
        return Err(anyhow!(
            "yt-dlp playlist failed: {}",
            extract_error_message(&stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_playlist_dump(&stdout))
}

fn parse_playlist_dump(stdout: &str) -> (String, Vec<PlaylistEntry>) {
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

    (playlist_title, entries)
}

pub struct PlaylistEntry {
    pub id: String,
    pub title: String,
    pub url: String,
    pub duration: Option<f64>,
}

/// yt-dlp `--download-archive` line prefix for a channel URL, or `None` when
/// the platform's archive id format is not known well enough to rely on
/// `--break-on-existing`. Callers that get `None` should fall back to a full
/// (non-incremental) listing and dedupe in their own seen-set.
pub fn archive_extractor_prefix(url: &str) -> Option<&'static str> {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_lowercase()))?;
    if host.contains("youtube.com") || host.contains("youtu.be") {
        Some("youtube")
    } else {
        // Other platforms' archive id formats are not stable enough across
        // yt-dlp versions to rely on --break-on-existing; callers fall back to
        // a full listing + seen-set dedupe, which stays correct either way.
        None
    }
}

/// Incremental playlist listing for channel polling. Writes a temporary
/// yt-dlp archive built from `seen_ids` and passes `--break-on-existing` so
/// yt-dlp stops enumerating as soon as it reaches a video that was already
/// seen. Tolerates the non-zero exit yt-dlp returns when it breaks early and
/// still parses whatever entries were emitted before the stop.
pub async fn get_playlist_info_incremental(
    ytdlp: &Path,
    url: &str,
    seen_ids: &[String],
    extractor_prefix: &str,
    playlist_end: u32,
) -> anyhow::Result<(String, Vec<PlaylistEntry>)> {
    if is_youtube_url(url) {
        yt_rate_limiter().acquire().await;
    }

    let archive_path =
        std::env::temp_dir().join(format!("omniget-chan-archive-{}.txt", std::process::id()));
    {
        let mut content = String::with_capacity(seen_ids.len() * 24);
        for id in seen_ids {
            if !id.is_empty() {
                content.push_str(extractor_prefix);
                content.push(' ');
                content.push_str(id);
                content.push('\n');
            }
        }
        std::fs::write(&archive_path, content)?;
    }

    let mut args = vec![
        "--flat-playlist".to_string(),
        "--dump-json".to_string(),
        "--no-warnings".to_string(),
        "--encoding".to_string(),
        "utf-8".to_string(),
        "--socket-timeout".to_string(),
        "30".to_string(),
        "--retries".to_string(),
        "3".to_string(),
        "--extractor-retries".to_string(),
        "3".to_string(),
        "--user-agent".to_string(),
        CHROME_UA.to_string(),
        "--lazy-playlist".to_string(),
        "--break-on-existing".to_string(),
        "--download-archive".to_string(),
        archive_path.to_string_lossy().to_string(),
        "--playlist-end".to_string(),
        playlist_end.to_string(),
    ];
    args.extend(js_runtime_args());
    if is_youtube_url(url) {
        args.push("--extractor-args".to_string());
        args.push("youtube:player_client=default".to_string());
    }
    args.extend(proxy_args());
    args.push(url.to_string());

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        crate::core::process::command(ytdlp)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await;

    let _ = std::fs::remove_file(&archive_path);

    let output = output
        .map_err(|_| anyhow!("Timeout fetching playlist (120s)"))?
        .map_err(|e| anyhow!("Failed to run yt-dlp: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let (title, entries) = parse_playlist_dump(&stdout);

    if !output.status.success() && entries.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_lower = stderr.to_lowercase();
        // An empty result with a clean "stopped due to --break-on-existing"
        // is the normal "no new videos" case, not a failure.
        if stderr_lower.contains("--break-on-existing")
            || stderr_lower.contains("already been recorded")
            || stderr_lower.contains("stopping further")
        {
            return Ok((title, entries));
        }
        if stderr_lower.contains("http error 429") {
            rate_limit_429_increment();
        }
        return Err(anyhow!(
            "yt-dlp playlist failed: {}",
            extract_error_message(&stderr)
        ));
    }

    Ok((title, entries))
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
    std::fs::write(path, content)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn download_video(
    ytdlp: &Path,
    url: &str,
    output_dir: &Path,
    quality_height: Option<u32>,
    progress: mpsc::Sender<ProgressUpdate>,
    download_mode: Option<&str>,
    format_id: Option<&str>,
    filename_template: Option<&str>,
    referer: Option<&str>,
    cancel_token: CancellationToken,
    cookie_file: Option<&Path>,
    concurrent_fragments: u32,
    download_subtitles: bool,
    extra_flags: &[String],
    audio_format: Option<&str>,
) -> anyhow::Result<DownloadResult> {
    let _timer_start = std::time::Instant::now();

    if is_youtube_url(url) {
        yt_rate_limiter().acquire().await;
    }

    let _ = progress.send(ProgressUpdate::percent(-1.0)).await;
    let download_started_at = std::time::SystemTime::now();

    let mode = download_mode.unwrap_or("auto");
    let is_audio_only = mode == "audio";
    let (ffmpeg_available, ffmpeg_location, aria2c_path) = tokio::join!(
        crate::core::ffmpeg::is_ffmpeg_available(),
        find_ffmpeg_location_cached(),
        crate::core::dependencies::ensure_aria2c(),
    );

    let format_selector = if let Some(fid) = format_id {
        if let Some(h) = quality_height.filter(|h| *h > 0) {
            let fallback = match mode {
                "audio" => "ba/b".to_string(),
                "mute" => format!("bv*[height<={}]/bv*/b", h),
                _ => {
                    if ffmpeg_available {
                        format!(
                            "bv*[height<={}]+ba[ext=m4a]/bv*[height<={}]+ba/b[height<={}]/b",
                            h, h, h
                        )
                    } else {
                        format!("b[height<={}]/bv*[height<={}]/b", h, h)
                    }
                }
            };
            format!("{}/{}", fid, fallback)
        } else {
            fid.to_string()
        }
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
                            "bv*[height<={}]+ba[ext=m4a]/bv*[height<={}]+ba/b[height<={}]/b",
                            h, h, h
                        ),
                        _ => "bv*+ba[ext=m4a]/bv*+ba/b".to_string(),
                    }
                } else {
                    tracing::warn!("[yt-dlp] ffmpeg not available, using fallback format selector");
                    match quality_height {
                        Some(h) if h > 0 => format!("b[height<={}]/bv*[height<={}]/b", h, h),
                        _ => "b/bv*".to_string(),
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
        .unwrap_or_else(|| format!("%(title).{}s [%(id)s].%(ext)s", max_name));
    let output_template = output_dir.join(&template).to_string_lossy().to_string();

    std::fs::create_dir_all(output_dir)?;

    let explicit_cookie_header = has_explicit_cookie_header(extra_flags);
    let per_domain_cookies = if explicit_cookie_header || cookie_file.is_some() {
        None
    } else {
        per_domain_cookie_file(url)
    };
    let managed_only = managed_cookies_only();
    let allow_fallback = !managed_only
        && !explicit_cookie_header
        && cookie_file.is_none()
        && per_domain_cookies.is_none();

    let manual_cookie_header = if allow_fallback {
        manual_cookie_header_setting()
    } else {
        None
    };
    let manual_cookie_enabled = manual_cookie_header.is_some();
    let global_cookie_file = if allow_fallback && !manual_cookie_enabled {
        global_cookie_file()
    } else {
        None
    };

    let ext_cookies = if allow_fallback && !manual_cookie_enabled && global_cookie_file.is_none() {
        extension_cookie_file()
    } else {
        None
    };

    let had_global_cookie_file = global_cookie_file.is_some();
    let had_ext_cookies = ext_cookies.is_some();

    let effective_cookie_file = cookie_file
        .map(|p| p.to_path_buf())
        .or_else(|| per_domain_cookies.clone())
        .or_else(|| global_cookie_file.map(std::path::PathBuf::from))
        .or(ext_cookies);

    let cfb_setting = if !allow_fallback
        || manual_cookie_enabled
        || explicit_cookie_header
        || effective_cookie_file.is_some()
    {
        String::new()
    } else {
        cookies_from_browser_setting()
    };

    if let Some(dl_id) = log_hook::current_download_id() {
        let host = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()))
            .unwrap_or_else(|| url.to_string());
        let slug = log_hook::current_cookie_slug();
        let line = if explicit_cookie_header {
            "[cookies] using explicit Cookie header from extra flags".to_string()
        } else if cookie_file.is_some() {
            "[cookies] using caller-supplied cookie file".to_string()
        } else if let Some(ref p) = per_domain_cookies {
            let slug_label = slug.as_deref().unwrap_or("_default");
            format!(
                "[cookies] using managed cookie for {} (account: {}) → {}",
                host,
                slug_label,
                p.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            )
        } else if manual_cookie_enabled {
            format!(
                "[cookies] no managed cookie for {}; using manual Cookie header from settings",
                host
            )
        } else if had_global_cookie_file {
            format!(
                "[cookies] no managed cookie for {}; using global cookie file from settings",
                host
            )
        } else if had_ext_cookies {
            format!(
                "[cookies] no managed cookie for {}; using browser-extension cookies",
                host
            )
        } else if !cfb_setting.is_empty() {
            format!(
                "[cookies] no managed cookie for {}; using --cookies-from-browser {}",
                host, cfb_setting
            )
        } else {
            format!(
                "[cookies] no cookies available for {} — downloading without auth",
                host
            )
        };
        log_hook::emit_log(dl_id, &line);
    }

    let mut base_args = vec![
        "-f".to_string(),
        format_selector,
        "--encoding".to_string(),
        "utf-8".to_string(),
        "--print".to_string(),
        "after_video:OMNIGET_FILEPATH:%(filepath)s".to_string(),
    ];
    base_args.extend(js_runtime_args());

    if mode == "audio" {
        let target_fmt = audio_format.unwrap_or("m4a");
        if format_id.is_none() && target_fmt == "m4a" {
            base_args.push("-S".to_string());
            base_args.push("+codec:aac:m4a".to_string());
        } else {
            base_args.push("-x".to_string());
            base_args.push("--audio-format".to_string());
            base_args.push(target_fmt.to_string());
        }
    }

    if format_id.is_none() && mode != "audio" && ffmpeg_available {
        base_args.push("--merge-output-format".to_string());
        base_args.push("mp4".to_string());
    }

    if let Some(ref_url) = referer {
        base_args.push("--referer".to_string());
        base_args.push(ref_url.to_string());
        base_args.push("--add-headers".to_string());
        base_args.push(format!("Referer:{}", ref_url));
    }

    if let Some(ext_headers) = ext_headers_for_url(url) {
        for (name, value) in ext_headers {
            let lower = name.to_lowercase();
            if lower == "referer" || lower == "cookie" || lower == "user-agent" {
                continue;
            }
            base_args.push("--add-headers".to_string());
            base_args.push(format!("{}:{}", name, value));
        }
    }

    if let Some(ref cf) = effective_cookie_file {
        base_args.push("--cookies".to_string());
        base_args.push(cf.to_string_lossy().to_string());
    }
    if let Some(ref cookie_header) = manual_cookie_header {
        append_cookie_header(&mut base_args, cookie_header);
    }

    if let Some(ref loc) = ffmpeg_location {
        base_args.push("--ffmpeg-location".to_string());
        base_args.push(loc.clone());
    }

    let effective_fragments = if is_youtube_url(url) {
        let rate_limit_count = rate_limit_429_count();
        let max_frags = if rate_limit_count >= 2 {
            2
        } else if rate_limit_count > 0 {
            4
        } else {
            8
        };
        concurrent_fragments.min(max_frags)
    } else {
        concurrent_fragments
    };
    base_args.push("-N".to_string());
    base_args.push(effective_fragments.to_string());

    if is_youtube_url(url) {
        base_args.push("--extractor-args".to_string());
        base_args.push("youtube:player_client=default".to_string());

        base_args.push("--throttled-rate".to_string());
        base_args.push("100K".to_string());

        base_args.push("--sleep-subtitles".to_string());
        base_args.push("5".to_string());
    }

    base_args.extend(["--buffer-size".to_string(), "16M".to_string()]);
    if !is_youtube_url(url) {
        base_args.extend(["--http-chunk-size".to_string(), "10M".to_string()]);
    }

    let mut use_aria2c = aria2c_path.is_some()
        && mode != "audio"
        && effective_cookie_file.is_none()
        && cfb_setting.is_empty()
        && !manual_cookie_enabled
        && !explicit_cookie_header;

    let effective_ua = ext_user_agent_for_url(url)
        .or_else(user_agent_setting)
        .unwrap_or_else(|| CHROME_UA.to_string());
    base_args.extend([
        "--no-check-certificate".to_string(),
        "--no-warnings".to_string(),
        "--no-mtime".to_string(),
        "--user-agent".to_string(),
        effective_ua,
        "--socket-timeout".to_string(),
        "30".to_string(),
        "--retries".to_string(),
        "5".to_string(),
        "--fragment-retries".to_string(),
        "5".to_string(),
        "--extractor-retries".to_string(),
        "3".to_string(),
        "--file-access-retries".to_string(),
        "3".to_string(),
        "--retry-sleep".to_string(),
        "exp=1:30".to_string(),
        "--trim-filenames".to_string(),
        max_name.to_string(),
        "--no-playlist".to_string(),
        "--newline".to_string(),
        "--progress-template".to_string(),
        "download:%(progress._percent_str)s|eta:%(progress.eta)s|spd:%(progress.speed)s|dl:%(progress.downloaded_bytes)s|tot:%(progress.total_bytes)s|est:%(progress.total_bytes_estimate)s".to_string(),
        "-o".to_string(),
        output_template,
        "--skip-unavailable-fragments".to_string(),
    ]);

    base_args.extend(proxy_args());
    base_args.extend(extra_flags.iter().cloned());

    if let Some(lang) = translate_metadata_lang() {
        base_args.push("--extractor-args".to_string());
        base_args.push(format!("youtube:lang={}", normalize_youtube_lang(&lang)));
    }

    if sponsorblock_enabled() && is_youtube_url(url) {
        let cats = sponsorblock_categories();
        let cat_arg = if cats.is_empty() {
            "default".to_string()
        } else {
            cats.join(",")
        };
        let flag = if sponsorblock_mode() == "mark" {
            "--sponsorblock-mark"
        } else {
            "--sponsorblock-remove"
        };
        base_args.push(flag.to_string());
        base_args.push(cat_arg);
    }

    if split_chapters_enabled() {
        base_args.push("--split-chapters".to_string());
    }

    if embed_metadata_enabled() {
        base_args.push("--embed-metadata".to_string());
    }

    if embed_thumbnail_enabled() {
        base_args.push("--embed-thumbnail".to_string());
        base_args.push("--convert-thumbnails".to_string());
        base_args.push("jpg".to_string());
    }

    if let Some(rate) = speed_limit_value() {
        base_args.push("--limit-rate".to_string());
        base_args.push(rate);
    }

    let frag_count = concurrent_fragments_value();
    if frag_count > 1 {
        base_args.push("--concurrent-fragments".to_string());
        base_args.push(frag_count.to_string());
    }

    if live_from_start_enabled() {
        base_args.push("--live-from-start".to_string());
        base_args.push("--no-part".to_string());
    }

    if cfg!(target_os = "windows") {
        base_args.push("--windows-filenames".to_string());
    }

    let include_auto_subs = include_auto_subs_setting();
    let mut selected_subtitle_lang: Option<String> = None;
    let mut should_download_subs = download_subtitles && rate_limit_429_count() < 2;
    if should_download_subs {
        match select_available_subtitle_lang(ytdlp, url, extra_flags, include_auto_subs).await {
            Ok(Some(lang)) => {
                if let Some(dl_id) = log_hook::current_download_id() {
                    log_hook::emit_log(
                        dl_id,
                        &format!("[subtitles] using selected subtitle language: {}", lang),
                    );
                }
                selected_subtitle_lang = Some(lang);
            }
            Ok(None) => {
                should_download_subs = false;
            }
            Err(err) => {
                should_download_subs = false;
                let line = format!(
                    "[subtitles] could not check subtitles; continuing without subtitles: {}",
                    err
                );
                tracing::warn!("{}", line);
                if let Some(dl_id) = log_hook::current_download_id() {
                    log_hook::emit_log(dl_id, &line);
                }
            }
        }
    }
    let subtitle_args = if should_download_subs {
        let mut args = vec!["--write-sub".to_string()];
        if include_auto_subs {
            args.push("--write-auto-sub".to_string());
        }
        let caption_locale = selected_subtitle_lang.unwrap_or_else(caption_locale_setting);
        args.extend([
            "--sub-lang".to_string(),
            caption_locale,
            "--sub-format".to_string(),
            "best".to_string(),
        ]);
        if !keep_vtt_setting() {
            args.extend(["--convert-subs".to_string(), "srt".to_string()]);
        }
        args
    } else {
        Vec::new()
    };

    let max_attempts: usize = 3;
    let mut extra_args: Vec<String> = Vec::new();
    let mut last_error = String::new();
    let mut use_subtitles = should_download_subs;
    let mut use_cfb = !cfb_setting.is_empty() && !explicit_cookie_header && !manual_cookie_enabled;
    let mut format_already_simplified = false;
    let mut last_was_429 = false;

    for attempt in 0..max_attempts {
        tracing::info!("[yt-dlp] download attempt {}/{}", attempt + 1, max_attempts);
        if cancel_token.is_cancelled() {
            tracing::debug!("[perf] download_video took {:?}", _timer_start.elapsed());
            anyhow::bail!("Download cancelled");
        }

        if attempt > 0 {
            let wait: u64 = if last_was_429 {
                match attempt {
                    1 => 3,
                    2 => 8,
                    _ => 15,
                }
            } else {
                1
            };
            tracing::info!(
                "[yt-dlp] retry {}/{} after {}s (429={})",
                attempt,
                max_attempts - 1,
                wait,
                last_was_429
            );
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            cleanup_part_files(output_dir).await;
        }

        let mut args = base_args.clone();

        if use_subtitles {
            args.extend(subtitle_args.iter().cloned());
        }

        if use_cfb {
            args.push("--cookies-from-browser".to_string());
            args.push(cfb_setting.clone());
        }

        if use_aria2c && !use_cfb {
            if let Some(ref a2_path) = aria2c_path {
                let conns = if is_youtube_url(url) {
                    effective_fragments.max(1)
                } else {
                    effective_fragments.clamp(8, 16)
                };
                args.push("--downloader".to_string());
                args.push(a2_path.to_string_lossy().to_string());
                args.push("--downloader-args".to_string());
                let aria2c_proxy = match crate::core::http_client::proxy_url() {
                    Some(url) => format!(" --all-proxy={}", url),
                    None => String::new(),
                };
                args.push(format!("aria2c:-x {} -k 1M -j {} --min-split-size=1M --file-allocation=none --optimize-concurrent-downloads=true --auto-file-renaming=false --summary-interval=1 --console-log-level=warn{}", conns, conns, aria2c_proxy));
            }
        }

        args.extend(extra_args.iter().cloned());
        args.push(url.to_string());

        let mut cmd = crate::core::process::command(ytdlp);
        cmd.args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("Failed to start yt-dlp: {}", e))?;
        let registered_download_id = log_hook::current_download_id();
        if let (Some(download_id), Some(pid)) = (registered_download_id, child.id()) {
            register_download_process(download_id, pid);
        }
        tracing::debug!(
            "[perf] download_video: yt-dlp process spawned at {:?} (attempt {})",
            _timer_start.elapsed(),
            attempt + 1
        );

        let _ = progress.send(ProgressUpdate::percent(-2.0)).await;

        let stdout = child.stdout.take().ok_or_else(|| anyhow!("No stdout"))?;
        let stderr_pipe = child.stderr.take().ok_or_else(|| anyhow!("No stderr"))?;

        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        let progress_tx = progress.clone();
        let captured_path: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
        let captured_path_writer = captured_path.clone();
        let log_id = log_hook::current_download_id();

        let line_reader = tokio::spawn(async move {
            let mut phase = 0u32;
            let mut max_reported = 0.0f64;
            let mut first_line_logged = false;
            let mut first_progress_logged = false;
            let mut authoritative_capture = false;
            let mut last_send = std::time::Instant::now();
            let throttle = std::time::Duration::from_millis(250);
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(id) = log_id {
                    log_hook::emit_log(id, &line);
                }
                if !first_line_logged {
                    first_line_logged = true;
                    tracing::debug!(
                        "[perf] download_video first_byte_time: {:?}",
                        _timer_start.elapsed()
                    );
                }
                if let Some(rest) = line.strip_prefix("OMNIGET_FILEPATH:") {
                    let final_path = rest.trim();
                    if !final_path.is_empty() && final_path != "NA" {
                        authoritative_capture = true;
                        let mut guard = captured_path_writer.lock().unwrap();
                        *guard = Some(PathBuf::from(final_path));
                    }
                    continue;
                }
                if let Some(dest) = parse_destination_line(&line) {
                    let dest_path = PathBuf::from(&dest);
                    let ext = dest_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let is_subtitle =
                        matches!(ext.as_str(), "vtt" | "srt" | "ass" | "ssa" | "sub" | "lrc");
                    if !is_subtitle && !authoritative_capture {
                        phase += 1;
                        let mut guard = captured_path_writer.lock().unwrap();
                        *guard = Some(dest_path);
                    }
                }
                if line.contains("[Merger]") {
                    let merging_progress = max_reported.max(95.0).min(98.0);
                    if merging_progress > max_reported {
                        max_reported = merging_progress;
                        let _ = progress_tx
                            .send(ProgressUpdate::percent(merging_progress))
                            .await;
                        last_send = std::time::Instant::now();
                    }
                    continue;
                }
                if let Some(pct) = parse_progress_line(&line) {
                    if !first_progress_logged && pct > 0.0 {
                        first_progress_logged = true;
                        tracing::debug!(
                            "[perf] download_video: first_progress > 0% at {:?}",
                            _timer_start.elapsed()
                        );
                    }
                    let eta = parse_eta_line(&line);
                    let speed = parse_speed_line(&line);
                    if let (Some(id), Some(e)) = (log_id, eta) {
                        record_eta(id, e);
                    }
                    if is_audio_only {
                        if pct >= 99.0 || last_send.elapsed() >= throttle {
                            let dl = parse_downloaded_bytes_line(&line);
                            let tot = parse_total_bytes_line(&line);
                            let _ = progress_tx
                                .send(ProgressUpdate::rich(pct, dl, tot, speed, eta))
                                .await;
                            last_send = std::time::Instant::now();
                        }
                    } else {
                        let adjusted = if phase <= 1 {
                            pct * 0.5
                        } else {
                            50.0 + pct * 0.5
                        };
                        if adjusted > max_reported
                            && (adjusted >= 99.0 || last_send.elapsed() >= throttle)
                        {
                            max_reported = adjusted;
                            let _ = progress_tx
                                .send(ProgressUpdate::rich(adjusted, None, None, speed, eta))
                                .await;
                            last_send = std::time::Instant::now();
                        }
                    }
                } else if line.trim_start().starts_with("download:") || line.contains("[download]")
                {
                    let dl = parse_downloaded_bytes_line(&line)
                        .or_else(|| parse_default_download_line(&line).map(|(d, _)| d as u64));
                    let speed = parse_speed_line(&line)
                        .or_else(|| parse_default_download_line(&line).map(|(_, s)| s));
                    if (dl.is_some() || speed.is_some()) && last_send.elapsed() >= throttle {
                        let _ = progress_tx
                            .send(ProgressUpdate::rich(0.0, dl, None, speed, None))
                            .await;
                        last_send = std::time::Instant::now();
                    }
                }
            }
        });

        let stderr_log_id = log_hook::current_download_id();
        let stderr_reader = tokio::spawn(async move {
            let mut buf = String::new();
            let stderr_buf = BufReader::new(stderr_pipe);
            let mut stderr_lines = stderr_buf.lines();
            while let Ok(Some(line)) = stderr_lines.next_line().await {
                if let Some(id) = stderr_log_id {
                    log_hook::emit_log(id, &line);
                }
                buf.push_str(&line);
                buf.push('\n');
            }
            buf
        });

        let status = tokio::select! {
            s = child.wait() => s.map_err(|e| anyhow!("yt-dlp process failed: {}", e))?,
            _ = cancel_token.cancelled() => {
                let _ = child.kill().await;
                if let Some(download_id) = registered_download_id {
                    unregister_download_process(download_id);
                }
                let _ = line_reader.await;
                let _ = stderr_reader.await;
                cleanup_part_files(output_dir).await;
                tracing::debug!("[perf] download_video took {:?}", _timer_start.elapsed());
                anyhow::bail!("Download cancelled");
            }
        };

        if let Some(download_id) = registered_download_id {
            unregister_download_process(download_id);
        }

        let _ = line_reader.await;
        let stderr_content = stderr_reader.await.unwrap_or_default();

        if status.success() {
            let _ = progress.send(ProgressUpdate::percent(100.0)).await;

            let file_path = {
                let guard = captured_path.lock().unwrap();
                guard.clone()
            };

            let file_path = match file_path {
                Some(p) if p.exists() => {
                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                    let is_audio_ext = matches!(
                        ext.to_lowercase().as_str(),
                        "m4a" | "mp3" | "ogg" | "opus" | "flac" | "aac" | "wav"
                    );
                    if is_audio_ext && !is_audio_only {
                        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                        let mp4_candidate = p.with_file_name(format!("{}.mp4", stem));
                        if mp4_candidate.exists() {
                            mp4_candidate
                        } else {
                            find_downloaded_file(output_dir, url).await.unwrap_or(p)
                        }
                    } else {
                        p
                    }
                }
                _ => find_downloaded_file(output_dir, url).await?,
            };
            if download_subtitles {
                let moved = ensure_subtitles_next_to_media(
                    output_dir,
                    &file_path,
                    download_started_at,
                    url,
                );
                if moved > 0 {
                    tracing::info!(
                        "[yt-dlp] moved {} subtitle file(s) next to {}",
                        moved,
                        file_path.display()
                    );
                }
            }

            let meta = std::fs::metadata(&file_path)?;
            tracing::debug!("[perf] download_video took {:?}", _timer_start.elapsed());
            return Ok(DownloadResult {
                file_path,
                file_size_bytes: meta.len(),
                duration_seconds: 0.0,
                torrent_id: None,
            });
        }

        last_error = stderr_content;
        let stderr_lower = last_error.to_lowercase();

        if attempt < max_attempts - 1 {
            if use_aria2c
                && (stderr_lower.contains("aria2") || stderr_lower.contains("external downloader"))
            {
                use_aria2c = false;
                tracing::warn!("[yt-dlp] aria2c failed, retrying with native downloader");
            }

            last_was_429 = stderr_lower.contains("http error 429");

            if last_was_429 {
                let is_subtitle_only_429 = last_error.lines().all(|line| {
                    let ll = line.to_lowercase();
                    !ll.contains("429") || ll.contains("subtitle")
                });

                if use_subtitles {
                    use_subtitles = false;
                    tracing::warn!(
                        "[yt-dlp] 429 detected, disabling subtitle download for remaining retries"
                    );
                }

                if is_subtitle_only_429 {
                    tracing::warn!("[yt-dlp] subtitle-only 429, retrying without subtitles (keeping current player_client)");
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                } else {
                    rate_limit_429_increment();
                    let sanitized_url = sanitize_log_line(url);
                    let player_client = if is_youtube_url(url) {
                        "default"
                    } else {
                        "n/a"
                    };
                    let cookies_enabled = use_cfb || effective_cookie_file.is_some();
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
                    tracing::warn!(
                        "[yt-dlp] rate limited (429), waiting {}s (base={}s + jitter={}s)",
                        wait_secs,
                        base_secs,
                        jitter_secs
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;

                    if is_youtube_url(url) {
                        base_args
                            .retain(|a| a != "--extractor-args" && !a.contains("player_client"));
                        extra_args
                            .retain(|a| a != "--extractor-args" && !a.contains("player_client"));
                        let client = match attempt {
                            0 => "youtube:player_client=mweb",
                            1 => "youtube:player_client=ios",
                            _ => "youtube:player_client=ios",
                        };
                        extra_args.push("--extractor-args".to_string());
                        extra_args.push(client.to_string());
                        tracing::warn!(
                            "[yt-dlp] 429 detected, rotating player_client to {}",
                            client
                        );
                    }
                }
            }

            if stderr_lower.contains("nsig") {
                base_args.retain(|a| a != "--extractor-args" && !a.contains("player_client"));
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

            if stderr_lower.contains("subtitle") && use_subtitles && !last_was_429 {
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

            if (stderr_lower.contains("403") || stderr_lower.contains("forbidden"))
                && referer.is_none()
                && !extra_args.contains(&"--referer".to_string())
            {
                let fallback = ext_referer_for_url(url).or_else(|| {
                    url::Url::parse(url).ok().and_then(|parsed| {
                        let host = parsed.host_str()?;
                        Some(format!("{}://{}/", parsed.scheme(), host))
                    })
                });
                if let Some(ref_url) = fallback {
                    tracing::info!("[yt-dlp] 403, adding fallback referer: {}", ref_url);
                    extra_args.push("--referer".to_string());
                    extra_args.push(ref_url.clone());
                    extra_args.push("--add-headers".to_string());
                    extra_args.push(format!("Referer:{}", ref_url));
                }
            }

            if ((stderr_lower.contains("could not") && stderr_lower.contains("cookie"))
                || stderr_lower.contains("cookies-from-browser")
                || stderr_lower.contains("failed to decrypt")
                || stderr_lower.contains("keyring")
                || stderr_lower.contains("permission denied"))
                && use_cfb
            {
                use_cfb = false;
                tracing::warn!("[yt-dlp] cookies-from-browser failed. Use the browser extension or set a cookie file in Settings.");
                COOKIE_ERROR_FLAG.store(true, std::sync::atomic::Ordering::Relaxed);
            }

            if (stderr_lower.contains("sign in") || stderr_lower.contains("login required"))
                && !use_cfb
                && effective_cookie_file.is_none()
            {
                tracing::warn!("[yt-dlp] login required. Install the browser extension and visit the site while logged in.");
            }

            if stderr_lower.contains("requested format") && stderr_lower.contains("not available")
                || stderr_lower.contains("ffmpeg") && stderr_lower.contains("not found")
                || stderr_lower.contains("postprocessing")
            {
                if format_already_simplified {
                    tracing::warn!(
                        "[yt-dlp] format/postprocessing error after simplification, giving up"
                    );
                    break;
                }

                base_args.retain(|a| a != "--extractor-args" && !a.contains("player_client"));
                extra_args.retain(|a| a != "--extractor-args" && !a.contains("player_client"));
                base_args.retain(|a| a != "--merge-output-format" && a != "mp4");

                if let Some(pos) = base_args.iter().position(|a| a == "-f") {
                    base_args.remove(pos + 1);
                    base_args.remove(pos);
                }
                tracing::warn!("[yt-dlp] format/postprocessing error on attempt {}, removed -f and player_client to use yt-dlp defaults", attempt + 1);
                format_already_simplified = true;
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

    tracing::debug!("[perf] download_video took {:?}", _timer_start.elapsed());
    Err(translate_ytdlp_error(&last_error))
}

async fn cleanup_part_files(dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".part") || name.ends_with(".ytdl") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn ensure_subtitles_next_to_media(
    output_dir: &Path,
    media_path: &Path,
    started_at: std::time::SystemTime,
    url: &str,
) -> usize {
    let Some(media_dir) = media_path.parent() else {
        return 0;
    };
    let Some(media_stem) = media_path.file_stem().and_then(|s| s.to_str()) else {
        return 0;
    };
    let media_stem = media_stem.to_string();
    let video_id = extract_id_from_url(url).unwrap_or_default();
    let cutoff = started_at
        .checked_sub(std::time::Duration::from_secs(5))
        .unwrap_or(started_at);
    let mut candidates = Vec::new();
    collect_subtitle_candidates(output_dir, cutoff, &mut candidates, 0);

    let mut moved = 0usize;
    for path in candidates {
        if path.parent() == Some(media_dir) {
            continue;
        }
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let matches_media = stem.starts_with(&media_stem)
            || media_stem.starts_with(stem)
            || (!video_id.is_empty() && name.contains(&video_id));
        if !matches_media {
            continue;
        }
        let Some(file_name) = path.file_name() else {
            continue;
        };
        let dest = unique_sidecar_path(media_dir.join(file_name));
        if move_file_best_effort(&path, &dest) {
            moved += 1;
        }
    }
    moved
}

fn collect_subtitle_candidates(
    dir: &Path,
    cutoff: std::time::SystemTime,
    out: &mut Vec<PathBuf>,
    depth: usize,
) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_subtitle_candidates(&path, cutoff, out, depth + 1);
            continue;
        }
        if !path.is_file() || !is_subtitle_path(&path) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.len() == 0 {
            continue;
        }
        if meta.modified().map(|m| m >= cutoff).unwrap_or(false) {
            out.push(path);
        }
    }
}

fn is_subtitle_path(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "vtt" | "srt" | "ass" | "ssa" | "sub" | "lrc" | "ttml"
    )
}

fn unique_sidecar_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("subtitle");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    for idx in 1..1000 {
        let file_name = if ext.is_empty() {
            format!("{} ({})", stem, idx)
        } else {
            format!("{} ({}).{}", stem, idx, ext)
        };
        let candidate = parent.join(file_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path
}

fn move_file_best_effort(from: &Path, to: &Path) -> bool {
    if std::fs::rename(from, to).is_ok() {
        return true;
    }
    if std::fs::copy(from, to).is_ok() {
        let _ = std::fs::remove_file(from);
        return true;
    }
    false
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

    if lower.contains("errno 22")
        && (lower.contains("textiowrapper")
            || lower.contains("encoding=")
            || lower.contains("exception ignored"))
    {
        return anyhow!(
            "Console encoding error (non-UTF-8 locale). Update yt-dlp in Settings → Dependencies, or run `chcp 65001` in a terminal and reopen the app."
        );
    }

    if lower.contains("http error 429") {
        return anyhow!("Server returned error 429 (too many requests). Try again later.");
    }
    if lower.contains("http error 403") || lower.contains("forbidden") {
        return anyhow!("Access denied (403). The video may be private or region-restricted.");
    }
    if lower.contains("sign in to confirm")
        || lower.contains("login required")
        || stderr.contains("请先登录")
        || stderr.contains("需要登录")
        || stderr.contains("登录后可")
        || stderr.contains("仅登录用户")
        || lower.contains("this video is only available") && lower.contains("members")
    {
        return anyhow!(
            "This video requires login. Import cookies for this site in Settings → Cookies, then retry."
        );
    }
    if lower.contains("nsig extraction failed") || lower.contains("nsig") {
        return anyhow!("Video extraction failed. Update yt-dlp or try again.");
    }
    if lower.contains("cannot parse data")
        || lower.contains("please report this issue")
        || (lower.contains("confirm you are on the latest version") && lower.contains("yt-dlp"))
    {
        return anyhow!(
            "yt-dlp extractor is broken for this site. Update yt-dlp in Settings → Dependencies, then retry."
        );
    }
    if lower.contains("requested format") && lower.contains("not available") {
        return anyhow!(
            "Requested format is not available. The download will retry with a compatible format."
        );
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
    if lower.contains("ffmpeg") && (lower.contains("not found") || lower.contains("no such file")) {
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
        if trimmed.len() > 300 {
            &trimmed[..300]
        } else {
            trimmed
        }
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

    if let Some(pct) = parse_aria2c_progress(line) {
        return Some(pct);
    }

    let body = if let Some(rest) = line.strip_prefix("download:") {
        rest
    } else if line.ends_with('%') {
        line
    } else {
        return None;
    };

    let pct_part = body.split('|').next()?.trim().trim_end_matches('%');
    let pct_str = pct_part.split_whitespace().last()?;
    pct_str.parse::<f64>().ok()
}

fn parse_aria2c_progress(line: &str) -> Option<f64> {
    if !line.starts_with("[#") {
        return None;
    }
    let open = line.find('(')?;
    let after = &line[open + 1..];
    let close = after.find("%)")?;
    after[..close].trim().parse::<f64>().ok()
}

fn template_field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let body = line.trim().strip_prefix("download:")?;
    for part in body.split('|') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix(key) {
            let rest = rest.trim();
            if rest.is_empty() || rest.eq_ignore_ascii_case("na") {
                return None;
            }
            return Some(rest);
        }
    }
    None
}

fn parse_eta_line(line: &str) -> Option<u64> {
    let raw = template_field(line, "eta:")?;
    raw.parse::<u64>()
        .ok()
        .or_else(|| raw.parse::<f64>().ok().map(|f| f.max(0.0) as u64))
}

fn parse_speed_line(line: &str) -> Option<f64> {
    let raw = template_field(line, "spd:")?;
    raw.parse::<f64>().ok().filter(|s| *s > 0.0)
}

fn parse_total_bytes_line(line: &str) -> Option<u64> {
    template_field(line, "tot:")
        .and_then(|v| v.parse::<f64>().ok())
        .or_else(|| template_field(line, "est:").and_then(|v| v.parse::<f64>().ok()))
        .filter(|v| *v > 0.0)
        .map(|v| v as u64)
}

fn parse_downloaded_bytes_line(line: &str) -> Option<u64> {
    template_field(line, "dl:")
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| *v >= 0.0)
        .map(|v| v as u64)
}

fn parse_size_token(token: &str) -> Option<f64> {
    let t = token.trim().trim_end_matches("/s").trim();
    let split = t.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let value: f64 = num.trim().parse().ok()?;
    let mult = match unit.trim() {
        "" | "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        "KB" | "kB" | "K" => 1000.0,
        "MB" | "M" => 1_000_000.0,
        "GB" | "G" => 1_000_000_000.0,
        "TB" | "T" => 1_000_000_000_000.0,
        _ => return None,
    };
    Some(value * mult)
}

/// Parse the default (non-template) yt-dlp download line used for live /
/// size-unknown streams, e.g. `[download]  2.87MiB at  506.63KiB/s (00:00:07)`.
/// Returns (downloaded_bytes, speed_bytes_per_sec).
fn parse_default_download_line(line: &str) -> Option<(f64, f64)> {
    let body = line.split("[download]").nth(1)?.trim();
    if body.contains('%') {
        return None;
    }
    let (size_part, rest) = body.split_once(" at ")?;
    let size = parse_size_token(size_part.trim())?;
    let speed_token = rest.trim().split_whitespace().next()?;
    let speed = parse_size_token(speed_token)?;
    Some((size, speed))
}

async fn find_downloaded_file(output_dir: &Path, url: &str) -> anyhow::Result<PathBuf> {
    let video_id = extract_id_from_url(url).unwrap_or_default();
    let media_extensions: &[&str] = &[
        "mp4", "mkv", "webm", "m4a", "mp3", "ogg", "opus", "flac", "avi", "mov", "ts", "m4v",
        "3gp", "aac", "wav",
    ];
    let now = std::time::SystemTime::now();
    let recency_limit = std::time::Duration::from_secs(1800);

    std::fs::create_dir_all(output_dir)?;
    let read_dir = std::fs::read_dir(output_dir)?;
    let mut candidates: Vec<(PathBuf, std::time::SystemTime, bool)> = Vec::new();

    for entry in read_dir.flatten() {
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

        if let Ok(meta) = entry.metadata() {
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

    if let Some((p, _, _)) = candidates.into_iter().next() {
        return Ok(p);
    }

    let fallback_limit = std::time::Duration::from_secs(120);
    let mut newest: Option<(PathBuf, std::time::SystemTime)> = None;
    if let Ok(entries) = std::fs::read_dir(output_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".part") || name.ends_with(".ytdl") || name.starts_with('.') {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                if meta.len() == 0 {
                    continue;
                }
                if let Ok(modified) = meta.modified() {
                    if now.duration_since(modified).unwrap_or_default() < fallback_limit {
                        if newest.as_ref().map_or(true, |(_, t)| modified > *t) {
                            newest = Some((path, modified));
                        }
                    }
                }
            }
        }
    }

    newest
        .map(|(p, _)| p)
        .ok_or_else(|| anyhow!(
            "Download reported success but no matching file appeared in {:?}. \
             This can happen on Windows with a non-UTF-8 console locale when the title contains non-Latin characters. \
             Try `chcp 65001` in a terminal before launching the app, or update yt-dlp in Settings → Dependencies.",
            output_dir
        ))
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

        if !has_video && !has_audio {
            continue;
        }

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

    if host.contains("youtube.com") && parsed.path().starts_with("/embed/") {
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
        return segments.last().map(|s| s.to_string());
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

    if host.contains("bilibili.com") || host == "b23.tv" {
        for seg in parsed.path().split('/').filter(|s| !s.is_empty()) {
            if (seg.starts_with("BV") && seg.len() >= 10)
                || (seg.starts_with("av") && seg.len() > 2)
            {
                return Some(seg.trim_end_matches('/').to_string());
            }
        }
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
    fn sha256sums_parsing() {
        let sums = "abc  not-ours\n\
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  yt-dlp.exe\n\
1111111111111111111111111111111111111111111111111111111111111111  yt-dlp_macos\n";
        assert_eq!(
            parse_sha256sums(sums, "yt-dlp.exe").as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        assert_eq!(
            parse_sha256sums(sums, "yt-dlp_macos").as_deref(),
            Some("1111111111111111111111111111111111111111111111111111111111111111")
        );
        assert_eq!(parse_sha256sums(sums, "yt-dlp_missing"), None);
        assert_eq!(parse_sha256sums("garbage line", "yt-dlp"), None);
    }

    #[test]
    fn sha256_hex_known_vector() {
        // SHA-256 of the empty input.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn ytdlp_release_base_per_channel() {
        assert!(ytdlp_release_base(YtdlpChannel::Stable).contains("yt-dlp/yt-dlp/releases"));
        assert!(ytdlp_release_base(YtdlpChannel::Nightly).contains("yt-dlp-nightly-builds"));
    }

    #[test]
    fn archive_prefix_youtube_only() {
        assert_eq!(
            archive_extractor_prefix("https://www.youtube.com/@chan/videos"),
            Some("youtube")
        );
        assert_eq!(
            archive_extractor_prefix("https://youtu.be/abc"),
            Some("youtube")
        );
        assert_eq!(
            archive_extractor_prefix("https://space.bilibili.com/123"),
            None
        );
        assert_eq!(archive_extractor_prefix("not a url"), None);
    }

    #[test]
    fn parse_speed_from_template() {
        let line = "download:  45.2%|eta:30|spd:1572864.0|dl:5242880|tot:11534336|est:NA";
        assert_eq!(parse_speed_line(line), Some(1572864.0));
        assert_eq!(parse_downloaded_bytes_line(line), Some(5242880));
        assert_eq!(parse_total_bytes_line(line), Some(11534336));
        assert_eq!(parse_eta_line(line), Some(30));
    }

    #[test]
    fn parse_total_falls_back_to_estimate() {
        let line = "download:  10.0%|eta:NA|spd:NA|dl:1000|tot:NA|est:9999.0";
        assert_eq!(parse_total_bytes_line(line), Some(9999));
        assert_eq!(parse_speed_line(line), None);
    }

    #[test]
    fn live_template_line_has_no_numeric_percent() {
        let line = "download:  N/A%|eta:NA|spd:518542.0|dl:3010560|tot:NA|est:NA";
        assert_eq!(parse_progress_line(line), None);
        assert_eq!(parse_downloaded_bytes_line(line), Some(3010560));
        assert_eq!(parse_speed_line(line), Some(518542.0));
    }

    #[test]
    fn parse_size_token_units() {
        assert_eq!(parse_size_token("1B"), Some(1.0));
        assert_eq!(parse_size_token("2KiB"), Some(2048.0));
        assert_eq!(parse_size_token("2.87MiB"), Some(2.87 * 1024.0 * 1024.0));
        assert_eq!(parse_size_token("506.63KiB/s"), Some(506.63 * 1024.0));
        assert_eq!(parse_size_token("garbage"), None);
    }

    #[test]
    fn parse_default_live_download_line() {
        let line = "[download]    2.87MiB at  506.63KiB/s (00:00:07) (frag 91/2097)";
        let (dl, spd) = parse_default_download_line(line).unwrap();
        assert!((dl - 2.87 * 1024.0 * 1024.0).abs() < 1.0);
        assert!((spd - 506.63 * 1024.0).abs() < 1.0);
    }

    #[test]
    fn parse_default_download_line_ignores_percent_and_destination() {
        assert_eq!(
            parse_default_download_line("[download]  45.2% of 10.00MiB at 1.00MiB/s"),
            None
        );
        assert_eq!(
            parse_default_download_line("[download] Destination: video.mp4"),
            None
        );
    }

    #[test]
    fn parse_progress_with_eta_field() {
        assert_eq!(parse_progress_line("download:  45.2%|eta:30"), Some(45.2));
    }

    #[test]
    fn parse_eta_extracts_seconds() {
        assert_eq!(parse_eta_line("download:  45.2%|eta:30"), Some(30));
    }

    #[test]
    fn parse_eta_na_returns_none() {
        assert_eq!(parse_eta_line("download:  45.2%|eta:NA"), None);
    }

    #[test]
    fn parse_eta_missing_returns_none() {
        assert_eq!(parse_eta_line("download:  45.2%"), None);
    }

    #[test]
    fn parse_eta_no_prefix_returns_none() {
        assert_eq!(parse_eta_line("  45.2%|eta:30"), None);
    }

    #[test]
    fn parse_progress_garbage_returns_none() {
        assert_eq!(parse_progress_line("[info] Writing video subtitles"), None);
    }

    #[test]
    fn parse_progress_aria2c_summary() {
        assert_eq!(
            parse_progress_line("[#1ce85c 35MiB/68MiB(50%) CN:8 DL:1.5MiB ETA:21s]"),
            Some(50.0)
        );
    }

    #[test]
    fn parse_progress_aria2c_decimal() {
        assert_eq!(
            parse_progress_line("[#abc 100MiB/200MiB(50.5%) CN:5 DL:2MiB]"),
            Some(50.5)
        );
    }

    #[test]
    fn parse_progress_aria2c_no_paren_returns_none() {
        assert_eq!(parse_progress_line("[#abc NOTICE]"), None);
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
        assert!(is_youtube_url(
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
        ));
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
    fn extract_id_youtube_embed() {
        assert_eq!(
            extract_id_from_url("https://www.youtube.com/embed/dQw4w9WgXcQ"),
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
        assert!(err.to_string().contains("Requested format"), "Got: {}", err);
        assert!(
            !err.to_string().contains("Video unavailable"),
            "Should not contain 'Video unavailable', got: {}",
            err
        );
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
